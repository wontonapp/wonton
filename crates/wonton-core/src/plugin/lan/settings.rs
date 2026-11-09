//! 管理接口 `GET`/`PUT /_wonton/api/lan`（ADR-0019 的“接口：App 和核心”）：当场检测，返回端口开在哪里、可以选的
//! 网卡和开了局域网端口的插件；改选择时核对、存下，按新地址重建，做完再返回。

use std::net::IpAddr;
use std::sync::Arc;

use wonton_protocol::{
    LanAddress, LanChoice, LanInterface, LanInterfaceKind, LanPlugin, LanProblem, LanSettings,
};

use super::monitor::Detected;
use super::select::{self, Bound, Kind};
use super::webui::lan_port;
use crate::l10n;
use crate::plugin::languages::Languages;
use crate::plugin::manager::{InstallError, PluginManager, PluginStatus};

fn address(bound: &Bound) -> LanAddress {
    LanAddress {
        address: bound.ip.to_string(),
        interface: bound.interface.clone(),
        // 插件网络只有 IPv4：IPv6 地址上的连接由 docker-proxy 转发，插件看到的客户端是网桥的网关。
        proxied: bound.ip.is_ipv6(),
    }
}

/// 宣告地址：不在虚拟机的网卡上，连接经宿主转发，插件看到的客户端地址不准。
fn announced(ip: &IpAddr) -> LanAddress {
    LanAddress {
        address: ip.to_string(),
        interface: None,
        proxied: true,
    }
}

fn kind(kind: Kind) -> LanInterfaceKind {
    match kind {
        Kind::Ethernet => LanInterfaceKind::Ethernet,
        Kind::Wifi => LanInterfaceKind::Wifi,
        Kind::Bridge => LanInterfaceKind::Bridge,
        Kind::Tunnel => LanInterfaceKind::Tunnel,
        Kind::WireGuard => LanInterfaceKind::WireGuard,
        Kind::Other => LanInterfaceKind::Other,
    }
}

impl PluginManager {
    /// 当场检测一次（结果的处理同定期检测），返回局域网设置。
    pub async fn lan_settings(&self, langs: &Languages) -> Result<LanSettings, InstallError> {
        let detected = self.lan_pass(langs).await?;
        self.lan_view(&detected, langs)
    }

    /// 核对主人的新选择，存下，按新地址重建开了局域网端口的插件，做完再返回新的设置。重建失败的插件标为异常，
    /// 不让请求失败：选择已经生效。在独立的任务里执行，请求断开了也照样做完。
    pub async fn set_lan_choice(
        self: &Arc<Self>,
        choice: LanChoice,
        langs: Languages,
    ) -> Result<LanSettings, InstallError> {
        let this = Arc::clone(self);
        crate::plugin::manager::detached(async move {
            this.check_lan_choice(&choice, &langs).await?;
            this.store.save_lan_choice(&choice)?;
            tracing::info!(?choice, "主人改了局域网设置");
            let detected = this.lan_pass(&langs).await?;
            this.lan_view(&detected, &langs)
        })
        .await
    }

    async fn check_lan_choice(
        &self,
        choice: &LanChoice,
        langs: &Languages,
    ) -> Result<(), InstallError> {
        if self.lan.announce.is_some() {
            return Err(InstallError::Conflict(l10n::text(
                langs,
                "plugin-settings-lan-announce-by-env",
                &[],
            )));
        }
        if self.lan.fixed.is_some() {
            return Err(InstallError::Conflict(l10n::text(
                langs,
                "plugin-settings-lan-fixed-by-env",
                &[],
            )));
        }
        let detected = self.detect_lan(langs).await?;
        if let Some(failure) = detected.failure {
            return Err(InstallError::LanUnavailable(l10n::text(
                langs,
                "plugin-settings-lan-detect-failed",
                &[("failure", failure.into())],
            )));
        }
        match select::check_choice(&detected.host, choice, langs) {
            Ok(_) => Ok(()),
            Err(Ok(message)) => Err(InstallError::Invalid(message)),
            Err(Err(problem)) => Err(InstallError::LanUnavailable(problem.message)),
        }
    }

    fn lan_view(
        &self,
        detected: &Detected,
        langs: &Languages,
    ) -> Result<LanSettings, InstallError> {
        let (addresses, announce_addresses, problem) = match &detected.result {
            Ok(plan) => (
                plan.bind.iter().map(address).collect(),
                if self.lan.announce.is_some() {
                    plan.announce.iter().map(announced).collect()
                } else {
                    plan.bind.iter().map(address).collect()
                },
                None,
            ),
            Err(problem) => (
                Vec::new(),
                Vec::new(),
                Some(LanProblem {
                    reason: problem.reason,
                    message: problem.message.clone(),
                }),
            ),
        };
        let interfaces = detected
            .host
            .candidates
            .iter()
            .map(|c| LanInterface {
                name: c.name.clone(),
                kind: kind(c.kind),
                addresses: c
                    .addresses
                    .iter()
                    .map(|a| {
                        address(&Bound {
                            ip: a.ip,
                            interface: Some(c.name.clone()),
                        })
                    })
                    .collect(),
                default_route: c.default_route,
            })
            .collect();
        Ok(LanSettings {
            choice: self.store.lan_choice()?.unwrap_or(LanChoice::Auto),
            fixed_by_environment: self.lan.fixed.is_some() || self.lan.announce.is_some(),
            announce_by_environment: self.lan.announce.is_some(),
            announce_addresses,
            addresses,
            problem,
            interfaces,
            plugins: self.lan_plugin_views(langs),
        })
    }

    fn lan_plugin_views(&self, langs: &Languages) -> Vec<LanPlugin> {
        let open = self.lan_plugins();
        let plugins = self.plugins.read().expect("插件表锁中毒");
        open.iter()
            .filter_map(|(id, spec)| {
                let entry = plugins.get(id)?;
                Some(LanPlugin {
                    id: id.to_string(),
                    title: langs.pick_owned(&entry.title),
                    ports: spec.ports.iter().map(lan_port).collect(),
                    problem: match &entry.status {
                        PluginStatus::Degraded { reason } => Some(reason.clone()),
                        _ => None,
                    },
                })
            })
            .collect()
    }
}
