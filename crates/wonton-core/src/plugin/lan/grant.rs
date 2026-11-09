//! 安装和预览时的局域网端口：开在哪里，和已装的插件冲不冲突（ADR-0019 的“端口冲突在预览里就说清”“安装确认
//! 醒目展示”）。被宿主自己的服务占着的端口核心看不到，由 Docker 在启动容器时报错。

use std::net::IpAddr;

use wonton_protocol::{CardLine, LanFailure, PortProtocol, PortSpec};

use super::LanGrant;
use super::monitor::Detected;
use super::ports;
use super::select::{Bound, Plan, Problem};
use super::webui::open_ports;
use crate::l10n;
use crate::plugin::languages::Languages;
use crate::plugin::manager::{InstallError, PluginManager};
use crate::plugin::manifest::{Manifest, Permission};
use crate::plugin::package::PluginPackage;

/// 要连一下试试的地址和端口：连一下只判得了 TCP，UDP 端口不放。
fn targets(addresses: &[IpAddr], ports: &[PortSpec]) -> Vec<(IpAddr, u16)> {
    addresses
        .iter()
        .flat_map(|address| {
            ports
                .iter()
                .filter(|port| port.protocol == PortProtocol::Tcp)
                .map(move |port| (*address, port.port))
        })
        .collect()
}

/// 安装预览里关于局域网端口的部分，同 [`crate::plugin::gpu::GpuPreview`]。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LanPreview {
    pub notices: Vec<CardLine>,
    pub blocked: Option<String>,
}

/// 端口开在哪块网卡的哪个地址上，如 `eno1 的 192.168.0.10、fd00::10`。
pub(super) fn bound_text(bound: &[Bound], langs: &Languages) -> String {
    let each: Vec<String> = bound
        .iter()
        .map(|b| match &b.interface {
            Some(interface) => l10n::text(
                langs,
                "plugin-settings-lan-bound",
                &[
                    ("interface", interface.as_str().into()),
                    ("ip", b.ip.to_string().into()),
                ],
            ),
            None => b.ip.to_string(),
        })
        .collect();
    l10n::list(langs, &each)
}

/// 手机要连的地址：启动器声明了宣告地址时写它，否则写绑定地址和网卡。
pub(super) fn describe(plan: &Plan, langs: &Languages) -> String {
    if plan.has_announce() {
        let each: Vec<String> = plan.announce.iter().map(ToString::to_string).collect();
        return l10n::list(langs, &each);
    }
    bound_text(&plan.bind, langs)
}

fn emphasized(text: String) -> CardLine {
    CardLine {
        text,
        emphasis: true,
    }
}

impl PluginManager {
    /// 申请了 `network.lan` 的插件的端口开在哪里：当场检测一次。选不出地址时装不了（`lan_unavailable`），端口被
    /// 已装的插件占着时冲突。开关关着、没有要开的端口时不检测也不查冲突；没申请的返回 `None`，不碰 Docker。
    pub(in crate::plugin) async fn lan_grant(
        &self,
        package: &PluginPackage,
        lan_webui: bool,
        langs: &Languages,
    ) -> Result<Option<LanGrant>, InstallError> {
        let manifest = &package.manifest;
        if manifest.lan.is_none() {
            return Ok(None);
        }
        let ports = open_ports(package, lan_webui);
        if ports.is_empty() {
            return Ok(Some(self.lan_grant_on(
                package,
                lan_webui,
                Vec::new(),
                Vec::new(),
            )));
        }
        if let Some(conflict) = self.lan_conflict(manifest, &ports, langs) {
            return Err(InstallError::Conflict(conflict));
        }
        let plan = self
            .detect_lan(langs)
            .await?
            .result
            .map_err(|p| self.unavailable(p, langs))?;
        let addresses: Vec<IpAddr> = plan.bind.iter().map(|b| b.ip).collect();
        // 启动容器那一步 Docker 也会拒绝，但在拉了镜像、建了网络之后；这里先查，装之前就知道。
        let published = self.engine.published_ports().await?;
        let taken = ports::clashes(&published, Some(manifest.id.as_str()), &addresses, &ports);
        if let Some(taken) = taken.first() {
            return Err(InstallError::PortTaken(taken.describe(langs, false)));
        }
        Ok(Some(self.lan_grant_on(
            package,
            lan_webui,
            addresses,
            plan.announce,
        )))
    }

    /// 装不了的原因。改选网卡能解决的，提示去局域网设置；检测失败、跑在桌面虚拟机里或者地址由环境变量指定时
    /// 改选没用，不提示。
    pub(super) fn unavailable(&self, problem: Problem, langs: &Languages) -> InstallError {
        let error = InstallError::LanUnavailable(problem.message);
        let unfixable = matches!(
            problem.reason,
            LanFailure::DetectionFailed | LanFailure::DesktopVm
        );
        if unfixable || self.lan.fixed.is_some() {
            return error;
        }
        error.with_note(
            langs,
            &l10n::text(langs, "plugin-settings-lan-note-reselect", &[]),
        )
    }

    /// 端口开在 `addresses` 上、告诉手机 `announce`。插件网络不能出网时另用一个网络发布：Docker 不在内部网络上发布端口，
    /// 也不报错。
    pub(in crate::plugin) fn lan_grant_on(
        &self,
        package: &PluginPackage,
        lan_webui: bool,
        addresses: Vec<IpAddr>,
        announce: Vec<IpAddr>,
    ) -> LanGrant {
        let manifest = &package.manifest;
        let ports = open_ports(package, lan_webui);
        let egress = manifest.permissions.contains(&Permission::NetworkEgress);
        LanGrant {
            addresses,
            announce,
            network: (!egress && !ports.is_empty())
                .then(|| self.cfg.instance.lan_network(&manifest.id.slug())),
            ports,
            webui: lan_webui,
        }
    }

    /// 不用开关就开的端口每个一行，写明开在哪块网卡的哪个地址上；再一行说明局域网的连接不经核心登录。要开关的端口
    /// 在 `InstallPreview::lan_webui` 里，主人打开了才开，这里不检查。
    pub(in crate::plugin) async fn lan_preview(
        &self,
        package: &PluginPackage,
        langs: &Languages,
    ) -> Result<LanPreview, InstallError> {
        let ports = open_ports(package, false);
        if ports.is_empty() {
            return Ok(LanPreview::default());
        }
        let detected = self.detect_lan(langs).await?;
        let result = &detected.result;
        let place = result.as_ref().ok().map(|plan| describe(plan, langs));
        let mut notices: Vec<CardLine> = ports
            .iter()
            .map(|p| {
                let line = |id: &str| {
                    l10n::text(
                        langs,
                        id,
                        &[
                            ("service", p.service.as_str().into()),
                            ("key", p.key().into()),
                        ],
                    )
                };
                emphasized(match &place {
                    Some(place) => l10n::text(
                        langs,
                        "plugin-settings-lan-port-line",
                        &[
                            ("service", p.service.as_str().into()),
                            ("key", p.key().into()),
                            ("place", place.as_str().into()),
                        ],
                    ),
                    None => line("plugin-settings-lan-port-line-plain"),
                })
            })
            .collect();
        notices.push(emphasized(l10n::text(
            langs,
            "plugin-settings-lan-direct-notice",
            &[],
        )));
        let Some(plan) = result.as_ref().ok() else {
            let blocked = self
                .lan_conflict(&package.manifest, &ports, langs)
                .or_else(|| {
                    result
                        .as_ref()
                        .err()
                        .map(|problem| self.unavailable(problem.clone(), langs).to_string())
                });
            return Ok(LanPreview { notices, blocked });
        };
        let addresses: Vec<IpAddr> = plan.bind.iter().map(|b| b.ip).collect();
        let published = self.engine.published_ports().await?;
        let blocked = self
            .lan_conflict(&package.manifest, &ports, langs)
            .or(self.docker_clash(package, &addresses, &ports, &published, langs));
        if blocked.is_none() {
            notices.extend(
                self.unheard_by_host(&detected, &addresses, &ports, &published, langs)
                    .await,
            );
        }
        Ok(LanPreview { notices, blocked })
    }

    /// Docker 里占着的端口挡下来：现在就知道装不上。
    fn docker_clash(
        &self,
        package: &PluginPackage,
        addresses: &[IpAddr],
        ports: &[PortSpec],
        published: &[ports::PublishedPort],
        langs: &Languages,
    ) -> Option<String> {
        let taken = ports::clashes(
            published,
            Some(package.manifest.id.as_str()),
            addresses,
            ports,
        );
        taken.first().map(|taken| taken.describe(langs, false))
    }

    /// 宿主上的程序占着的端口：在宿主网络里连一下试试，只提示不挡，Docker 看不到这些。
    async fn unheard_by_host(
        &self,
        detected: &Detected,
        addresses: &[IpAddr],
        ports: &[PortSpec],
        published: &[ports::PublishedPort],
        langs: &Languages,
    ) -> Vec<CardLine> {
        // 绑的是通配地址时，宿主上的程序可能只占着某块网卡的具体地址，试探要走检测到的地址。
        let probed: Vec<IpAddr> = if addresses.iter().any(IpAddr::is_unspecified) {
            detected
                .host
                .candidates
                .iter()
                .flat_map(|candidate| &candidate.addresses)
                .map(|address| address.ip)
                .filter(IpAddr::is_ipv4)
                .collect()
        } else {
            addresses.to_vec()
        };
        match self
            .lan
            .detector
            .listen(
                &self.engine,
                &self.cfg.instance.lan_probe(),
                self.cfg.instance.core_labels(),
                &targets(&probed, ports),
            )
            .await
        {
            Ok(heard) => ports::unheard(&heard, published)
                .into_iter()
                .map(|taken| emphasized(taken.describe(langs, true)))
                .collect(),
            // 漏报可能：别装作“没人占”。
            Err(reason) => vec![emphasized(l10n::text(
                langs,
                "plugin-install-lan-port-probe-failed",
                &[("reason", reason.into())],
            ))],
        }
    }

    /// 端口被已装（或正在装）的别的插件开着时的说明。别的插件的开关关着的端口不算。
    pub(super) fn lan_conflict(
        &self,
        manifest: &Manifest,
        ports: &[PortSpec],
        langs: &Languages,
    ) -> Option<String> {
        let plugins = self.plugins.read().expect("插件表锁中毒");
        plugins
            .iter()
            .filter(|(id, _)| **id != manifest.id)
            .find_map(|(id, entry)| {
                let listed = entry.listed.as_ref()?;
                let other = listed.open_ports(listed.lan_webui);
                let taken = ports.iter().find(|p| {
                    other
                        .iter()
                        .any(|o| o.port == p.port && o.protocol == p.protocol)
                })?;
                Some(l10n::text(
                    langs,
                    "plugin-settings-lan-conflict-taken",
                    &[("key", taken.key().into()), ("id", id.to_string().into())],
                ))
            })
    }
}
