//! 对账与应用客户端模型：按期望状态写配置、建删容器，配置或证书变了才让入口重读，失败的回滚。

use super::error::config_error;
use super::target::Target;
use super::{
    CONNECTOR_COMPONENT, ENTRY_COMPONENT, EntryContainers, EntryError, LAN_FORWARD_COMPONENT,
};
use crate::entry_settings::EntrySettings;
use crate::l10n;
use crate::plugin::docker::EngineError;
use crate::plugin::languages::Languages;
use wonton_protocol::{GateState, PublicSwitchState};

impl EntryContainers {
    /// 根据插件开关重写 HAProxy 模型、让局域网转发跟上，再让入口容器热加载。
    pub async fn apply_clients_model(
        &self,
        settings: &EntrySettings,
        public_access: bool,
        public_labels: Vec<String>,
        lan_labels: Vec<String>,
    ) -> Result<(), EntryError> {
        let _guard = self.reconcile.lock().await;
        let mut applied_model = self.operation.lock().await;
        if self.core_container.is_none() {
            return Err(EntryError::CoreNotInContainer);
        }
        if public_access && !public_labels.is_empty() && settings.registration.is_some() {
            return Err(config_error(
                "应用公网入口",
                anyhow::anyhow!("家名 frpc 目前只转发联邦域名，不能转发插件子域名"),
            ));
        }
        let core = self.core_container.as_deref().expect("已检查核心容器");
        let project = self.project(core).await?;
        // 有插件开着局域网入口才要 `lanfwd`；建了以后入口一直接着（见 `networks`）。
        let link = if lan_labels.is_empty() {
            None
        } else {
            self.lan_link(&project, true).await?
        };
        // 先把局域网转发容器建好再写配置、重载：入口的 `frontend lan` 绑的是
        // `entry-lanfwd.wonton.internal`，那个名字解析不出来时新 worker 会卡在 bind 上，重载一直
        // “还没开始回应”（2026-11-28 开发栈实测）。
        self.follow_lan_forward(&project, link.as_ref()).await?;
        let (_, networks) = self.networks(core).await?;
        // 证书还没换到新后缀时先按旧名字服务（和 `write_config` 同一条规则）。
        let names = crate::entry_certificate::published_names(
            &self.install_dir.join("entry/certs/site.pem"),
        );
        let domain = crate::entry_certificate::served_suffix(&names, &settings.domain);
        let mut model = crate::entry_haproxy::Model::federation_only(&domain, &domain);
        model.public = public_access;
        model.public_labels = if public_access {
            public_labels
        } else {
            Vec::new()
        };
        model.lan_labels = lan_labels;
        model.lan_forward = link.as_ref().map(|link| link.forward.to_string());
        model.connectors = vec![networks.connector];
        if applied_model.as_ref() == Some(&model) {
            self.reconcile_inner(Some(settings), public_access).await?;
            return Ok(());
        }
        let snapshot = crate::entry_compose::ModelSnapshot::read(&self.install_dir)
            .map_err(|error| config_error("读取旧入口配置", error))?;
        let applied = async {
            crate::entry_compose::write_model(&self.install_dir, &model)
                .map_err(|error| config_error("重写入口配置", error))?;
            crate::entry_compose::prepare_permissions(&self.install_dir)
                .map_err(|error| config_error("入口文件权限", error))?;
            self.reconcile_inner(Some(settings), public_access).await?;
            self.reload(&self.reload_addr).await
        }
        .await;
        if let Err(error) = applied {
            *applied_model = None;
            let mut failures = Vec::new();
            if let Err(cause) = snapshot.restore(&self.install_dir) {
                failures.push(format!("恢复配置失败：{cause:#}"));
            } else if let Err(cause) = self.reload(&self.reload_addr).await {
                failures.push(format!("重载旧配置失败：{cause}"));
            }
            return if failures.is_empty() {
                Err(error)
            } else {
                Err(config_error(
                    "应用入口配置及回滚",
                    anyhow::anyhow!("{error}；{}", failures.join("；")),
                ))
            };
        }
        *applied_model = Some(model);
        Ok(())
    }

    fn cert_digest(&self) -> Option<u64> {
        use std::hash::{Hash, Hasher};
        let pem = std::fs::read(self.install_dir.join("entry/certs/site.pem")).ok()?;
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        pem.hash(&mut hasher);
        Some(hasher.finish())
    }

    /// 证书内容变过（含同名续期）：上次加载的不是现在这份。
    fn cert_changed(&self) -> bool {
        let current = self.cert_digest();
        *self.loaded_cert.lock().expect("证书摘要锁") != current
    }

    fn remember_cert(&self) {
        let current = self.cert_digest();
        *self.loaded_cert.lock().expect("证书摘要锁") = current;
    }

    /// 按目标对账：配置按实际网段和已发布的证书写，缺的容器建、多的删、镜像换了重建、网络没接上就接上，
    /// 配置或证书变了才让入口重读，重载失败的下一轮再试。
    ///
    /// 要不要入口看的是实际有没有要服务的名字和证书，不是设置文件在不在：“不要名字”的安装没有设置文件，
    /// 主人在 App 里配的自己的域名写在 `entry/domain.json`，一样要有入口、一样要续期。
    pub async fn reconcile(
        &self,
        settings: Option<&EntrySettings>,
        public: bool,
    ) -> Result<(), EntryError> {
        let _guard = self.reconcile.lock().await;
        self.reconcile_locked(settings, public).await.map(|_| ())
    }

    /// [`Self::reconcile`] 的本体：返回这一轮有没有重写入口配置（连接器和公网证书列表都要跟着开关撤/建）。
    async fn reconcile_locked(
        &self,
        settings: Option<&EntrySettings>,
        public: bool,
    ) -> Result<bool, EntryError> {
        let mut applied_model = self.operation.lock().await;
        *applied_model = None;
        let target = Target::resolve(&self.install_dir, settings)?;
        // 文件缺了（旧版坏掉的安装）、证书换过（要按新证书重算名字）或公网证书列表和总开关对不上
        // （崩在写盘和重载之间、文件被改坏了）就按设置重写一份；否则只对容器，别把客户端开关写的标签覆盖成只有联邦。
        // 没有设置文件的装法没有第二个写入者，每轮都按 `domain.json` 和已发布的证书写。
        let missing = !self.install_dir.join("entry/haproxy.cfg").is_file();
        let cert_changed = self.cert_changed();
        // 列表为空 ⇔ 开关关着；对不上说明文件不是这一版开关写的。
        let switch_drift = self.public_crt_list_empty().unwrap_or(false) != !public;
        self.reconcile_target(
            target.as_ref(),
            settings.is_none() || missing || cert_changed || switch_drift,
            public,
        )
        .await
    }

    /// 下发一次总开关：撤/建连接器、清空/恢复公网证书列表，并让入口重读配置。逐道如实报结果。
    ///
    /// 没有入口设置文件（“不要名字”的装法）也有入口：名字来自 App 里配的自己的域名，照常对账。
    /// 同一个值重复下发不动入口（对账看文件内容，没变就不重写）。
    ///
    /// 固定原因的失败按 App 的语言写（`langs`），动态的 Docker 错误原样附在后面。
    pub async fn apply_public_access(
        &self,
        settings: Option<&EntrySettings>,
        public: bool,
        langs: &Languages,
    ) -> PublicSwitchState {
        let _guard = self.reconcile.lock().await;
        let changed = match self.reconcile_locked(settings, public).await {
            Ok(changed) => changed,
            Err(error) => return self.report_switch(&error.to_string()),
        };
        let connector = match self.connector_running().await {
            Ok(running) if running == public => GateState::Done,
            Ok(running) => GateState::Failed {
                detail: l10n::text(
                    langs,
                    if running {
                        "clients-gate-connector-still-running"
                    } else {
                        "clients-gate-connector-not-running"
                    },
                    &[],
                ),
            },
            Err(error) => GateState::Failed {
                detail: error.to_string(),
            },
        };
        let mut crt_list = match self.public_crt_list_empty() {
            Ok(empty) if empty != public => GateState::Done,
            Ok(empty) => GateState::Failed {
                detail: l10n::text(
                    langs,
                    if empty {
                        "clients-gate-public-list-still-empty"
                    } else {
                        "clients-gate-public-list-not-cleared"
                    },
                    &[],
                ),
            },
            Err(error) => GateState::Failed {
                detail: error.to_string(),
            },
        };
        // 配置没变时对账不会重载入口，这里补一次：上一次可能崩在写盘和重载之间。
        if !changed
            && self.running(ENTRY_COMPONENT).await.unwrap_or(false)
            && let Err(error) = self.reload(&self.reload_addr).await
        {
            crt_list = GateState::Failed {
                detail: l10n::text(
                    langs,
                    "clients-gate-entry-not-reloaded",
                    &[("error", error.to_string().into())],
                ),
            };
        }
        PublicSwitchState {
            connector,
            crt_list,
            core: GateState::Done,
        }
    }

    /// 客户端开关先 `write_model` 再走这里：模型里带着标签，所以不能再按“只有联邦”重写。
    async fn reconcile_inner(
        &self,
        settings: Option<&EntrySettings>,
        public: bool,
    ) -> Result<(), EntryError> {
        let target = Target::resolve(&self.install_dir, settings)?;
        self.reconcile_target(target.as_ref(), false, public)
            .await
            .map(|_| ())
    }

    /// 按目标对容器和配置对账：缺的建、多的删、镜像换了重建、网络没接上就接上。
    async fn reconcile_target(
        &self,
        target: Option<&Target>,
        write: bool,
        public: bool,
    ) -> Result<bool, EntryError> {
        let Some(target) = target else {
            // 没有要服务的名字和证书：容器都删掉，不去动网络；下次起来是新的。
            self.reload_pending
                .store(false, std::sync::atomic::Ordering::SeqCst);
            for component in [LAN_FORWARD_COMPONENT, ENTRY_COMPONENT, CONNECTOR_COMPONENT] {
                self.engine.remove_container(&self.name(component)).await?;
            }
            return Ok(false);
        };
        let Some(core) = &self.core_container else {
            // 要入口、又跑在宿主上：没有 compose 给的那些网络。
            return Err(EntryError::CoreNotInContainer);
        };
        let (project, networks) = self.networks(core).await?;
        // 配置一写下去（哪怕只写出一半）就先记着：后面任何容器动作失败都不能把这笔账丢掉。
        let changed = if write {
            match self.write_config(target, &networks.connector, public) {
                Ok(changed) => changed,
                Err(error) => {
                    self.reload_pending
                        .store(true, std::sync::atomic::Ordering::SeqCst);
                    return Err(error);
                }
            }
        } else {
            false
        };
        if changed {
            self.reload_pending
                .store(true, std::sync::atomic::Ordering::SeqCst);
        }
        let cert_changed = self.cert_changed();
        let existed = self
            .engine
            .inspect_container(&self.name(ENTRY_COMPONENT))
            .await?
            .is_some();
        let (entry, connector) = Self::wanted(target, public);
        let mut entry_started = false;
        for (wanted, component) in [(entry, ENTRY_COMPONENT), (connector, CONNECTOR_COMPONENT)] {
            if !wanted {
                self.engine.remove_container(&self.name(component)).await?;
                continue;
            }
            let side = self.side(component, &networks, target)?;
            let created = self.ensure(&side, &project).await?;
            if component == ENTRY_COMPONENT && created {
                entry_started = true;
            }
        }
        // 刚起/重建的入口自己会读当前证书和配置，但这一轮要重载时得等重载成功才算数：
        // 失败后配置和证书都可能不再变化，靠 `reload_pending` 跨轮重试，直到成功或容器重建。
        let pending = self
            .reload_pending
            .load(std::sync::atomic::Ordering::SeqCst);
        // 刚起/重建的进程启动时就读了当前配置和证书，不用再发一遍 reload：主机满载时重建后立刻
        // 重载的窗口里主进程还没开始回应，重试风暴会让每一轮都对不上（2026-11-28 开发栈实测）。
        let reload = (changed || cert_changed || pending) && existed && entry && !entry_started;
        if entry_started {
            self.remember_cert();
            self.reload_pending
                .store(false, std::sync::atomic::Ordering::SeqCst);
        }
        if reload {
            if let Err(error) = self.reload(&self.reload_addr).await {
                self.reload_pending
                    .store(true, std::sync::atomic::Ordering::SeqCst);
                return Err(error);
            }
            self.reload_pending
                .store(false, std::sync::atomic::Ordering::SeqCst);
            self.remember_cert();
        }
        Ok(changed)
    }

    /// 按已发布的证书写入口配置：证书还不认识新后缀时继续服务旧名字（少一次入口断开），
    /// 新证书发布后的下一轮对账才换。返回配置有没有变。
    fn write_config(
        &self,
        target: &Target,
        connector: &str,
        public: bool,
    ) -> Result<bool, EntryError> {
        let names = crate::entry_certificate::published_names(
            &self.install_dir.join("entry/certs/site.pem"),
        );
        let domain = crate::entry_certificate::served_suffix(&names, &target.domain);
        crate::entry_compose::sync(
            &self.install_dir,
            &crate::entry_compose::Config {
                domain: &domain,
                connector,
                registration: target.registration.as_ref(),
                relay_ca: target.relay_ca.as_deref(),
                public,
            },
        )
        .map_err(|error| {
            EntryError::Docker(EngineError::Rejected {
                context: "写入口配置".to_owned(),
                message: error.to_string(),
            })
        })
    }
}
