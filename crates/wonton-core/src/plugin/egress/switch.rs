//! 第一次配上或者彻底取消代理（ADR-0023 的“改地址和开关代理”）：插件的环境变量要变，只能重建出网插件的容器。数据不动，
//! 失败的插件标为异常并写明原因，下次核对时再试。只改地址时不走这里：插件拿到的是出口容器的地址，不用变。
//!
//! Bot 插件沿用同一台设备，令牌从旧容器读回（插件规范的“Matrix Bot”）：核心不单独保存令牌，它只在 Docker 的容器配置里。
//! 先读后删，读不到就不动容器、标为异常，不悄悄签新设备：换设备要重新建加密身份。

use super::provider::provides_proxy;
use crate::plugin::health_gate::Gate;
use wonton_protocol::EgressRestart;

use crate::l10n;
use crate::plugin::languages::Languages;
use crate::plugin::manager::{Deployed, Entry, InstallError, PluginManager, PluginStatus};
use crate::plugin::manifest::{Permission, PluginId};
use crate::plugin::package::PluginPackage;
use crate::plugin::render::{Deployment, render};

/// 找哪里引用了令牌时代替它。令牌不会长这样，插件也不会写出这串字。
pub(in crate::plugin) const PLACEHOLDER: &str = "wonton-matrix-token-to-read-back";

/// 容器的一个环境变量里引用了令牌，令牌前后是这些字。
#[derive(Debug, PartialEq, Eq)]
pub(in crate::plugin) struct TokenUse {
    pub(in crate::plugin) container: String,
    pub(in crate::plugin) variable: String,
    pub(in crate::plugin) before: String,
    pub(in crate::plugin) after: String,
}

impl TokenUse {
    /// 从旧容器的环境变量里取出令牌。
    pub(in crate::plugin) fn read(&self, env: &[String]) -> Option<String> {
        let value = env.iter().find_map(|line| {
            let (name, value) = line.split_once('=')?;
            (name == self.variable).then_some(value)
        })?;
        let token = value
            .strip_prefix(self.before.as_str())?
            .strip_suffix(self.after.as_str())?;
        (!token.is_empty()).then(|| token.to_owned())
    }
}

/// 用占位值渲染出来的部署计划里，哪些环境变量引用了令牌。写在 `command`、`configs` 这些地方的读不回来。
pub(in crate::plugin) fn token_uses(
    plan: &Deployment,
    langs: &Languages,
) -> Result<Vec<TokenUse>, String> {
    let mut uses = Vec::new();
    for container in &plan.containers {
        let service = &container.service;
        let in_files = container
            .files
            .iter()
            .any(|file| String::from_utf8_lossy(&file.data).contains(PLACEHOLDER));
        let mut rest = container.body.clone();
        let env = rest.env.take().unwrap_or_default();
        let elsewhere = serde_json::to_string(&rest).map_err(|e| e.to_string())?;
        if in_files || elsewhere.contains(PLACEHOLDER) {
            return Err(l10n::text(
                langs,
                "plugin-settings-egress-token-unreadable",
                &[("service", service.as_str().into())],
            ));
        }
        for line in &env {
            let Some((variable, value)) = line.split_once('=') else {
                continue;
            };
            if let Some((before, after)) = value.split_once(PLACEHOLDER) {
                uses.push(TokenUse {
                    container: container.name.clone(),
                    variable: variable.to_owned(),
                    before: before.to_owned(),
                    after: after.to_owned(),
                });
            }
        }
    }
    Ok(uses)
}

impl PluginManager {
    /// 出口容器按设置建好或者删掉，再重建环境变量和设置对不上的插件。核心启动时和主人改了设置后调用。
    pub async fn apply_egress(&self) -> Result<(), InstallError> {
        let _applying = self.egress.applying.lock().await;
        self.reconcile_relay().await?;
        let configured = self.egress_config()?.url.is_some();
        for id in self.egress_pending(configured)? {
            self.switch_egress(&id).await;
        }
        Ok(())
    }

    /// 配上（`configured`）或者取消代理时要重启的插件。
    pub fn egress_restarts(
        &self,
        configured: bool,
        langs: &Languages,
    ) -> Result<Vec<EgressRestart>, InstallError> {
        let pending = self.egress_pending(configured)?;
        let plugins = self.plugins.read().expect("插件表锁中毒");
        Ok(pending
            .iter()
            .filter_map(|id| {
                Some(EgressRestart {
                    id: id.to_string(),
                    title: langs.pick_owned(&plugins.get(id)?.title),
                })
            })
            .collect())
    }

    /// 声明了出网、容器的环境变量和设置对不上的插件。
    pub(super) fn egress_pending(&self, configured: bool) -> Result<Vec<PluginId>, InstallError> {
        let declared: Vec<PluginId> = {
            let plugins = self.plugins.read().expect("插件表锁中毒");
            plugins
                .iter()
                .filter(|(_, entry)| goes_out(entry))
                .map(|(id, _)| id.clone())
                .collect()
        };
        let mut pending = Vec::new();
        for id in declared {
            if self
                .store
                .load(&id)?
                .is_some_and(|record| record.proxied != configured)
            {
                pending.push(id);
            }
        }
        Ok(pending)
    }

    async fn switch_egress(&self, id: &PluginId) {
        if self.explicitly_stopped(id).unwrap_or(true) || self.recovery_pending(id) {
            return;
        }
        // 正在安装、卸载或者按新地址重建的不碰：下次核对时再做。
        if !self.begin_rebuild(id) {
            return;
        }
        let langs = self.owner_languages();
        let result = self.rebuild_for_egress(id, &langs).await;
        self.lan.end(id);
        let mut plugins = self.plugins.write().expect("插件表锁中毒");
        if let Some(entry) = plugins.get_mut(id)
            && let Some(listed) = entry.listed.clone()
        {
            *entry = match result {
                Ok(deployed) => {
                    tracing::info!(plugin = %id, "插件已按新的代理设置重建");
                    Entry::running(listed, deployed)
                }
                Err(e) => {
                    let reason = crate::l10n::text(
                        &langs,
                        "plugin-settings-egress-rebuild-failed",
                        &[("error", e.to_string().into())],
                    );
                    tracing::warn!(plugin = %id, %reason, "插件异常");
                    Entry::new(listed, PluginStatus::Degraded { reason }, None)
                }
            };
        }
        drop(plugins);
        self.changed();
    }

    async fn rebuild_for_egress(
        &self,
        id: &PluginId,
        langs: &Languages,
    ) -> Result<Deployed, InstallError> {
        let mut record = self
            .store
            .load(id)?
            .ok_or_else(|| InstallError::not_installed(langs, id))?;
        let mut package = PluginPackage::with_files(
            &record.source,
            record.files.clone(),
            &self.owner_languages(),
        )?;
        package.origin = record.origin.clone();
        let lan = self.lan_grant(&package, record.lan_webui, langs).await?;
        // 要迁就先预留+按新网段渲染：渲染失败放掉预留，旧网络和旧容器一个不动。
        let (pending, reservation) = match self.prepare_migration(&package.manifest).await? {
            Some((pending, reservation)) => (Some(pending), Some(reservation)),
            None => (None, None),
        };
        let mut ctx = self
            .rebuild_context(&package, &record, lan, pending.as_ref())
            .await?;
        // Bot 的 Matrix 变量和总线地址也要填：Bot 服务的容器引用了它们，令牌从旧容器读回。
        self.fill_bot_vars(&package, &mut ctx, langs).await?;
        let plan = render(&package, &ctx, &self.owner_languages())?;
        if let (Some(pending), Some(reservation)) = (&pending, reservation) {
            self.apply_migration(
                &package.manifest,
                pending,
                reservation,
                ctx.egress.is_some(),
                &self.owner_languages(),
            )
            .await?;
        }
        self.replace_containers(&ctx, plan, Gate::StartPeriod)
            .await?;
        record.proxied = ctx.egress.is_some();
        self.store.save_metadata(&record)?;
        self.reconcile_running(&package, &record).await
    }
}

fn goes_out(entry: &Entry) -> bool {
    entry.listed.as_ref().is_some_and(|listed| {
        let permissions = &listed.manifest.permissions;
        permissions.contains(&Permission::NetworkEgress) && !provides_proxy(&listed.manifest)
    })
}

pub(in crate::plugin) fn cannot_read(langs: &Languages, reason: String) -> InstallError {
    InstallError::Unhealthy(l10n::text(
        langs,
        "plugin-settings-egress-token-read-back",
        &[
            ("reason", reason.into()),
            (
                "reinstall",
                l10n::text(langs, "plugin-settings-egress-token-reinstall", &[]).into(),
            ),
        ],
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn token_use(before: &str, after: &str) -> TokenUse {
        TokenUse {
            container: "c".to_owned(),
            variable: "TOKEN".to_owned(),
            before: before.to_owned(),
            after: after.to_owned(),
        }
    }

    #[test]
    fn the_token_is_cut_out_of_whatever_surrounds_it() {
        let env = ["OTHER=x".to_owned(), "TOKEN=Bearer abc;end".to_owned()];
        assert_eq!(
            token_use("Bearer ", ";end").read(&env).as_deref(),
            Some("abc")
        );
        assert_eq!(token_use("Basic ", "").read(&env), None);
        assert_eq!(token_use("", "").read(&["TOKEN=".to_owned()]), None);
        assert_eq!(token_use("", "").read(&["NOPE=1".to_owned()]), None);
    }
}
