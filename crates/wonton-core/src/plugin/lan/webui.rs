//! 开关 `lan_webui`（ADR-0023 的“网页的局域网端口”）：有两种局域网端口，开了别人就能冒充主人或者抢先注册，默认不开，
//! 主人看过风险后打开。开关存在插件的记录里，改开关时重建开端口或插值用到地址的容器，数据不动。
//!
//! 哪些端口要开关，只看插件包：写了 `lan.client` 时，全部局域网端口；`sso: proxy` 用请求头认用户而 `webui.service`
//! 不是只信任核心时，这个服务的局域网端口；`sso: none` 把网页的端口列进 `lan.ports` 时，这个端口。其余的局域网端口身份
//! 由插件自己验证（ADR-0019），装好就开。

use std::sync::Arc;

use wonton_protocol::{LanPort, LanProtocol, LanWebuiOffer, PluginDetail, PortProtocol, PortSpec};

use super::select::Plan;
use crate::l10n;
use crate::plugin::core_address;
use crate::plugin::languages::Languages;
use crate::plugin::listed::Listed;
use crate::plugin::manager::{Deployed, Entry, InstallError, PluginManager, detached};
use crate::plugin::manifest::{Manifest, PluginId, Sso};
use crate::plugin::package::PluginPackage;
use crate::plugin::proxy_auth;

/// 要开关打开才开的局域网端口。
/// 网页的局域网端口的风险：决定 [`Switched::offer`] 里给 App 的说明用哪条。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Risk {
    Impersonate,
    Register,
    Client,
}

impl Risk {
    fn text(self, langs: &Languages) -> String {
        let id = match self {
            Risk::Impersonate => "plugin-settings-lan-webui-risk-impersonate",
            Risk::Register => "plugin-settings-lan-webui-risk-register",
            Risk::Client => "plugin-settings-lan-webui-risk-client",
        };
        l10n::text(langs, id, &[])
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Switched {
    pub ports: Vec<PortSpec>,
    risk: Risk,
}

impl Switched {
    pub fn offer(&self, enabled: bool, langs: &Languages) -> LanWebuiOffer {
        LanWebuiOffer {
            ports: self.ports.iter().map(lan_port).collect(),
            risk: self.risk.text(langs),
            enabled,
            client: None,
            client_problem: None,
        }
    }
}

pub(super) fn lan_port(port: &PortSpec) -> LanPort {
    LanPort {
        port: port.port,
        protocol: match port.protocol {
            PortProtocol::Tcp => LanProtocol::Tcp,
            PortProtocol::Udp => LanProtocol::Udp,
            PortProtocol::Other => LanProtocol::Other,
        },
    }
}

/// 网关用请求头告诉应用主人是谁，而应用不是只信任核心：能连上应用的都能冒充主人。
pub fn impersonable(package: &PluginPackage) -> bool {
    let m = &package.manifest;
    let Some(webui) = m.webui.as_ref().filter(|w| w.sso == Sso::Proxy) else {
        return false;
    };
    if !proxy_auth::uses_identity(m) {
        return false;
    }
    let env = package.raw_environment(&webui.service);
    let wide = env.contains("0.0.0.0/0") || env.contains("::/0");
    wide || !(env.contains(core_address::VAR) || env.contains(core_address::CIDR_VAR))
}

/// 这个插件包要开关的端口；没有时为空。
pub fn switched(package: &PluginPackage) -> Option<Switched> {
    let m = &package.manifest;
    if let Some(lan) = m.lan.as_ref().filter(|lan| lan.client.is_some()) {
        return Some(Switched {
            ports: lan.ports.clone(),
            risk: Risk::Client,
        });
    }
    let webui = m.webui.as_ref()?;
    let ports = m.lan.iter().flat_map(|lan| &lan.ports);
    let (ports, risk): (Vec<PortSpec>, _) = if impersonable(package) {
        let ports = ports.filter(|p| p.service == webui.service);
        (ports.cloned().collect(), Risk::Impersonate)
    } else if webui.sso == Sso::None {
        let ports = ports.filter(|p| {
            p.service == webui.service && p.port == webui.port && p.protocol == PortProtocol::Tcp
        });
        (ports.cloned().collect(), Risk::Register)
    } else {
        return None;
    };
    (!ports.is_empty()).then_some(Switched { ports, risk })
}

/// `lan.ports` 里开关是 `enabled` 时开着的端口。
pub fn open_ports_of(
    manifest: &Manifest,
    switched: Option<&Switched>,
    enabled: bool,
) -> Vec<PortSpec> {
    let closed = switched.filter(|_| !enabled);
    let ports = manifest.lan.iter().flat_map(|lan| &lan.ports);
    ports
        .filter(|p| closed.is_none_or(|c| !c.ports.contains(p)))
        .cloned()
        .collect()
}

pub fn open_ports(package: &PluginPackage, enabled: bool) -> Vec<PortSpec> {
    open_ports_of(&package.manifest, switched(package).as_ref(), enabled)
}

impl PluginManager {
    /// 改开关，重建完返回详情。在独立的任务里执行，请求断开了也照样做完。
    pub async fn set_lan_webui(
        self: &Arc<Self>,
        id: PluginId,
        enabled: bool,
        langs: Languages,
    ) -> Result<PluginDetail, InstallError> {
        let this = Arc::clone(self);
        detached(async move {
            this.switch_lan_webui(&id, enabled, &langs).await?;
            this.judged_detail(&id, &langs)
                .await
                .ok_or_else(|| InstallError::not_installed(&langs, &id))
        })
        .await
    }

    async fn switch_lan_webui(
        &self,
        id: &PluginId,
        enabled: bool,
        langs: &Languages,
    ) -> Result<(), InstallError> {
        let _pass = self.lan.pass.lock().await;
        self.change_lan_webui(id, enabled, langs).await
    }

    /// 调用方拿着 `lan.pass`。
    pub(super) async fn change_lan_webui(
        &self,
        id: &PluginId,
        enabled: bool,
        langs: &Languages,
    ) -> Result<(), InstallError> {
        let listed = {
            let plugins = self.plugins.read().expect("插件表锁中毒");
            plugins.get(id).and_then(|entry| entry.listed.clone())
        };
        let listed = listed.ok_or_else(|| InstallError::not_installed(langs, id))?;
        if listed.lan_switched.is_none() {
            return Err(InstallError::Invalid(l10n::text(
                langs,
                "plugin-settings-lan-webui-no-switch",
                &[("id", id.to_string().into())],
            )));
        }
        if listed.lan_webui == enabled && !self.lan.is_degraded(id) {
            return Ok(());
        }
        let plan = self.addresses_for(&listed, enabled, langs).await?;
        if !self.begin_rebuild(id) {
            return Err(InstallError::Conflict(l10n::text(
                langs,
                "plugin-settings-lan-webui-busy",
                &[("id", id.to_string().into())],
            )));
        }
        let result = self.apply_lan_webui(id, enabled, &plan, langs).await;
        let outcome = match result {
            Ok(deployed) => {
                tracing::info!(plugin = %id, enabled, "主人改了网页的局域网端口开关");
                self.lan_webui_applied(id, enabled, deployed);
                Ok(())
            }
            // 主人要关的端口不能开回去：别的服务不健康、Docker 出错时保持关，标为异常，定期检测接着关。
            Err(e) if !enabled => {
                self.set_listed_lan_webui(id, false);
                self.lan_failed(
                    id,
                    l10n::text(
                        langs,
                        "plugin-settings-lan-webui-close-failed",
                        &[("error", e.to_string().into())],
                    ),
                );
                Err(e)
            }
            Err(e) => {
                // 比如端口被宿主的服务占着：退回原来的样子，开关不算改成。
                match self
                    .apply_lan_webui(id, listed.lan_webui, &plan, langs)
                    .await
                {
                    Ok(deployed) => self.lan_webui_applied(id, listed.lan_webui, deployed),
                    Err(back) => self.lan_failed(
                        id,
                        l10n::text(
                            langs,
                            "plugin-settings-lan-webui-rollback-failed",
                            &[("error", back.to_string().into())],
                        ),
                    ),
                }
                Err(e)
            }
        };
        self.lan.end(id);
        outcome
    }

    /// 开关改成 `enabled` 后端口开在哪、告诉手机哪。有端口要开时，和新装一样核对冲突和地址。
    pub(in crate::plugin) async fn addresses_for(
        &self,
        listed: &Listed,
        enabled: bool,
        langs: &Languages,
    ) -> Result<Plan, InstallError> {
        let open = listed.open_ports(enabled);
        if open.is_empty() {
            return Ok(Plan::default());
        }
        if let Some(conflict) = self.lan_conflict(&listed.manifest, &open, langs) {
            return Err(InstallError::Conflict(conflict));
        }
        let detected = self.detect_lan(langs).await?;
        detected.result.map_err(|p| self.unavailable(p, langs))
    }

    /// 存下开关，重建 `lan.ports` 和插值用到地址的服务：开着的多发布端口，关了的连 `--lan` 网络也不接。
    async fn apply_lan_webui(
        &self,
        id: &PluginId,
        enabled: bool,
        plan: &Plan,
        langs: &Languages,
    ) -> Result<Deployed, InstallError> {
        let mut record = self
            .store
            .load(id)?
            .ok_or_else(|| InstallError::not_installed(langs, id))?;
        record.lan_webui = enabled;
        self.store.save_metadata(&record)?;
        let mut package = PluginPackage::with_files(&record.source, record.files.clone(), langs)?;
        package.origin = record.origin.clone();
        self.rebuild_lan(&package, &record, plan).await?;
        self.reconcile_running(&package, &record).await
    }

    fn lan_webui_applied(&self, id: &PluginId, enabled: bool, deployed: Deployed) {
        self.set_listed_lan_webui(id, enabled);
        let mut plugins = self.plugins.write().expect("插件表锁中毒");
        if let Some(entry) = plugins.get_mut(id)
            && let Some(listed) = entry.listed.clone()
        {
            *entry = Entry::running(listed, deployed);
        }
        drop(plugins);
        self.lan.clear_degraded(id);
        self.changed();
    }

    fn set_listed_lan_webui(&self, id: &PluginId, enabled: bool) {
        let mut plugins = self.plugins.write().expect("插件表锁中毒");
        if let Some(entry) = plugins.get_mut(id)
            && let Some(listed) = entry.listed.as_mut()
        {
            *listed = Arc::new(Listed {
                lan_webui: enabled,
                ..(**listed).clone()
            });
        }
    }
}
