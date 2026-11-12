//! 插件列表里的视图和网关用的路由：插件表现在是什么样，外面（App、网关）看到的就是什么。

use std::sync::Arc;

use serde::Serialize;

use super::super::entry::Entry;
use super::super::entry::Health;
use super::super::entry::PluginStatus;
use super::super::manifest::{LocalizedText, PluginId, Sso};
use super::super::proxy_auth::ProxyAuth;
use super::super::upstreams::Endpoint;
use super::PluginManager;

#[derive(Debug, Clone, Serialize)]
pub struct PluginView {
    pub id: PluginId,
    pub version: String,
    pub title: LocalizedText,
    pub status: PluginStatus,
    /// 只有运行中的插件才有。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub health: Option<Health>,
    pub url: Option<String>,
}

/// 网关查询用的路由信息。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Route {
    pub id: PluginId,
    pub status: PluginStatus,
    /// 核心可直接访问的插件网页地址。
    pub upstream: Option<Endpoint>,
    /// 清单 `webui` 指向的服务现在能用：插件整体异常时网关也照常转发。
    pub web_serving: bool,
    /// 网页响应是否要加跨源隔离策略头。
    pub cross_origin_isolated: bool,
    /// `sso: proxy` 时网关代加的凭据。
    pub proxy: Option<Arc<ProxyAuth>>,
    /// 请求路径是否精确匹配本次路由清单登记的 OIDC 登录回调。
    pub oidc_callback: bool,
}

impl PluginManager {
    pub fn route(&self, slug: &str, path: &str) -> Option<Route> {
        let plugins = self.plugins.read().expect("插件表锁中毒");
        plugins
            .iter()
            .find(|(id, _)| id.slug() == slug)
            .map(|(id, entry)| Route {
                id: id.clone(),
                status: entry.status.clone(),
                upstream: entry.upstream.clone(),
                web_serving: entry
                    .listed
                    .as_ref()
                    .and_then(|listed| listed.manifest.webui.as_ref())
                    .is_some_and(|webui| entry.serving.contains(&webui.service)),
                cross_origin_isolated: entry
                    .listed
                    .as_ref()
                    .and_then(|listed| listed.manifest.webui.as_ref())
                    .is_some_and(|webui| webui.cross_origin_isolated),
                proxy: entry.proxy.clone(),
                oidc_callback: entry
                    .listed
                    .as_ref()
                    .and_then(|listed| listed.manifest.webui.as_ref())
                    .filter(|webui| webui.sso == Sso::Oidc)
                    .and_then(|webui| webui.oidc.as_ref())
                    .is_some_and(|client| client.redirect_paths.iter().any(|p| p == path)),
            })
    }

    pub(in crate::plugin) fn view(&self, id: &PluginId, entry: &Entry) -> PluginView {
        PluginView {
            id: id.clone(),
            version: entry.version.clone(),
            title: entry.title.clone(),
            status: entry.status.clone(),
            health: entry.health,
            url: self.open_url(id, entry),
        }
    }

    /// 插件网页的 origin；用户打开的地址还要加清单声明的入口路径（见 [`Self::open_url`]）。
    pub(in crate::plugin) fn web_url(&self, id: &PluginId) -> String {
        self.cfg
            .public
            .origin(&self.cfg.public.plugin_host(&id.slug()))
    }

    /// 用户打开的插件网页地址：origin 加清单声明的入口路径（核心原样拼接，不再编码）。
    /// 网关按 Host 转发，代理路由不含入口路径；入口路径只决定打开时落在哪一页。
    pub(in crate::plugin) fn open_url(&self, id: &PluginId, entry: &Entry) -> Option<String> {
        entry
            .has_webui
            .then(|| format!("{}{}", self.web_url(id), entry.entry_path()))
    }
}
