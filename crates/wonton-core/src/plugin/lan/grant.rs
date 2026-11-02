//! 安装和预览时的局域网端口：开在哪里，和已装的插件冲不冲突（ADR-0019 的“端口冲突在预览里就说清”“安装确认
//! 醒目展示”）。被宿主自己的服务占着的端口核心看不到，由 Docker 在启动容器时报错。

use std::net::IpAddr;

use wonton_protocol::{CardLine, LanFailure};

use super::select::{Bound, Problem};
use super::{LanGrant, LanSpec};
use crate::plugin::manager::{InstallError, PluginManager};
use crate::plugin::manifest::{Manifest, Permission};

/// 安装预览里关于局域网端口的部分，同 [`crate::plugin::gpu::GpuPreview`]。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LanPreview {
    pub notices: Vec<CardLine>,
    pub blocked: Option<String>,
}

/// 地址和所在的网卡，如 `eno1 的 192.168.0.193、fd00::10`。
pub(super) fn describe(bound: &[Bound]) -> String {
    let each: Vec<String> = bound
        .iter()
        .map(|b| match &b.interface {
            Some(interface) => format!("{interface} 的 {}", b.ip),
            None => b.ip.to_string(),
        })
        .collect();
    each.join("、")
}

fn emphasized(text: String) -> CardLine {
    CardLine {
        text,
        emphasis: true,
    }
}

impl PluginManager {
    /// 申请了 `network.lan` 的插件的端口开在哪里：当场检测一次。选不出地址时装不了（`lan_unavailable`），端口被
    /// 已装的插件占着时冲突。没申请的返回 `None`，不碰 Docker。
    pub(in crate::plugin) async fn lan_grant(
        &self,
        manifest: &Manifest,
    ) -> Result<Option<LanGrant>, InstallError> {
        let Some(spec) = &manifest.lan else {
            return Ok(None);
        };
        if let Some(conflict) = self.lan_conflict(manifest, spec) {
            return Err(InstallError::Conflict(conflict));
        }
        let bound = self
            .detect_lan()
            .await?
            .result
            .map_err(|p| self.unavailable(p))?;
        let addresses = bound.iter().map(|b| b.ip).collect();
        Ok(Some(self.lan_grant_on(manifest, addresses)))
    }

    /// 装不了的原因。改选网卡能解决的，提示去局域网设置；检测失败或地址由环境变量指定时改选没用，不提示。
    fn unavailable(&self, problem: Problem) -> InstallError {
        let error = InstallError::LanUnavailable(problem.message);
        if problem.reason == LanFailure::DetectionFailed || self.lan.fixed.is_some() {
            return error;
        }
        error.with_note("可以在局域网设置里改选网卡或地址")
    }

    /// 端口开在 `addresses` 上。插件网络不能出网时另用一个网络发布：Docker 不在内部网络上发布端口，也不报错。
    pub(in crate::plugin) fn lan_grant_on(
        &self,
        manifest: &Manifest,
        addresses: Vec<IpAddr>,
    ) -> LanGrant {
        let egress = manifest.permissions.contains(&Permission::NetworkEgress);
        LanGrant {
            addresses,
            network: (!egress).then(|| self.cfg.instance.lan_network(&manifest.id.slug())),
        }
    }

    /// 每个端口一行，写明开在哪块网卡的哪个地址上；再一行说明局域网的连接不经核心登录。
    pub(in crate::plugin) async fn lan_preview(
        &self,
        manifest: &Manifest,
    ) -> Result<LanPreview, InstallError> {
        let Some(spec) = &manifest.lan else {
            return Ok(LanPreview::default());
        };
        let result = self.detect_lan().await?.result;
        let place = result.as_ref().ok().map(|bound| describe(bound));
        let mut notices: Vec<CardLine> = spec
            .ports
            .iter()
            .map(|p| {
                let port = format!("局域网端口：{} 的 {}", p.service, p.key());
                emphasized(match &place {
                    Some(place) => format!("{port}，开在 {place} 上"),
                    None => port,
                })
            })
            .collect();
        notices.push(emphasized(
            "局域网里的设备直接连这些端口，不经核心登录，身份由插件自己验证".to_owned(),
        ));
        let blocked = self
            .lan_conflict(manifest, spec)
            .or_else(|| Some(self.unavailable(result.err()?).to_string()));
        Ok(LanPreview { notices, blocked })
    }

    /// 端口被已装（或正在装）的别的插件占着时的说明。
    fn lan_conflict(&self, manifest: &Manifest, spec: &LanSpec) -> Option<String> {
        let plugins = self.plugins.read().expect("插件表锁中毒");
        plugins
            .iter()
            .filter(|(id, _)| **id != manifest.id)
            .find_map(|(id, entry)| {
                let other = entry.listed.as_ref()?.manifest.lan.as_ref()?;
                let taken = spec.ports.iter().find(|p| {
                    other
                        .ports
                        .iter()
                        .any(|o| o.port == p.port && o.protocol == p.protocol)
                })?;
                Some(format!("局域网端口 {} 已被插件 {id} 占用", taken.key()))
            })
    }
}
