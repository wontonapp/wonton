//! 插件表里的一项：插件现在是什么状态，网关要转发到哪里。

use std::sync::Arc;

use serde::Serialize;

use super::listed::Listed;
use super::manifest::{LocalizedText, PluginId, PluginKind};
use super::models::ModelsRoute;
use super::proxy_auth::ProxyAuth;
use super::upstreams::{Endpoint, McpTarget};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum PluginStatus {
    Installing,
    Running,
    Stopped,
    /// 已安装但状态异常（容器缺失、部分服务未运行、卸载中途失败等），原因如实给出。
    Degraded {
        reason: String,
    },
    Uninstalling,
    /// 正在换成新版本（见 [`super::upgrade`]）。
    Upgrading,
}

/// 运行中的插件健不健康，是最近一次检查的结果（见 [`super::watch`]）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Health {
    /// 每个长期运行的服务都有健康检查，而且都通过了。
    Healthy,
    /// 有长期运行的服务没有健康检查（或者检查时还没通过）：只知道它在运行，不能说它健康。
    Unknown,
}

impl Health {
    /// 所有长期运行的服务都健康才算健康。
    pub fn of(healthy: impl IntoIterator<Item = bool>) -> Self {
        if healthy.into_iter().all(|h| h) {
            Self::Healthy
        } else {
            Self::Unknown
        }
    }
}

pub(super) struct Entry {
    pub(super) version: String,
    pub(super) title: LocalizedText,
    /// `kind: system`：由安装器预装。
    pub(super) system: bool,
    pub(super) has_webui: bool,
    pub(super) status: PluginStatus,
    pub(super) health: Option<Health>,
    /// 列表和详情页显示的信息；记录读不出来的插件为空。
    pub(super) listed: Option<Arc<Listed>>,
    pub(super) upstream: Option<Endpoint>,
    /// 声明了 `provides.models`（不论状态）。
    pub(super) provides_models: bool,
    pub(super) provides_egress_proxy: bool,
    /// 运行中的模型网关的转发目标。
    pub(super) models: Option<ModelsRoute>,
    /// `sso: proxy` 时网关代加的凭据。
    pub(super) proxy: Option<Arc<ProxyAuth>>,
    /// 运行中时 MCP 服务的地址。
    pub(super) mcp: Option<McpTarget>,
    /// 这个异常是容器决定的，容器好了就恢复（见 [`super::watch`]）。
    pub(super) follows_containers: bool,
}

/// 部署或核对完成的插件：核心访问它网页的地址（是模型网关时还有网关接口的地址），和检查时的健康状况。
pub(super) struct Deployed {
    pub upstream: Option<Endpoint>,
    pub health: Health,
    pub models: Option<ModelsRoute>,
    pub proxy: Option<Arc<ProxyAuth>>,
    pub mcp: Option<McpTarget>,
    /// 直连宿主网络网页连不上。
    pub firewalled: bool,
}

impl Entry {
    pub(super) fn new(
        listed: Arc<Listed>,
        status: PluginStatus,
        upstream: Option<Endpoint>,
    ) -> Self {
        let manifest = &listed.manifest;
        Self {
            version: manifest.version.clone(),
            title: manifest.title.clone(),
            system: manifest.kind == PluginKind::System,
            has_webui: manifest.webui.is_some(),
            status,
            health: None,
            upstream,
            provides_models: manifest
                .provides
                .as_ref()
                .is_some_and(|p| p.models.is_some()),
            provides_egress_proxy: manifest
                .provides
                .as_ref()
                .is_some_and(|p| p.egress_proxy.is_some()),
            models: None,
            proxy: None,
            mcp: None,
            listed: Some(listed),
            follows_containers: false,
        }
    }

    pub(super) fn running(listed: Arc<Listed>, deployed: Deployed) -> Self {
        Self {
            health: Some(deployed.health),
            models: deployed.models,
            proxy: deployed.proxy,
            mcp: deployed.mcp,
            ..Self::new(listed, PluginStatus::Running, deployed.upstream)
        }
    }

    pub(super) fn stopped(listed: Arc<Listed>) -> Self {
        Self {
            follows_containers: true,
            ..Self::new(listed, PluginStatus::Stopped, None)
        }
    }

    /// 核对没过的插件：之后跟着容器走（见 [`super::watch`]），容器好了再核对一次就恢复。
    pub(super) fn containers_failed(listed: Arc<Listed>, reason: String) -> Self {
        Self {
            follows_containers: true,
            ..Self::new(listed, PluginStatus::Degraded { reason }, None)
        }
    }

    /// 记录或其中的插件包读不出来：只知道 ID（版本可能也不知道），如实标为异常，主人可以卸载它。
    pub(super) fn broken(id: &PluginId, version: String, reason: String) -> Self {
        Self {
            version,
            title: ["zh", "en"]
                .map(|lang| (lang.to_owned(), id.to_string()))
                .into(),
            system: false,
            has_webui: false,
            status: PluginStatus::Degraded { reason },
            health: None,
            upstream: None,
            provides_models: false,
            provides_egress_proxy: false,
            models: None,
            proxy: None,
            mcp: None,
            listed: None,
            follows_containers: false,
        }
    }

    pub(super) fn failed_with(&self, reason: &str) -> bool {
        matches!(&self.status, PluginStatus::Degraded { reason: r } if r == reason)
    }
}
