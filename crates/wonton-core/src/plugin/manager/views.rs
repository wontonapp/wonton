//! 插件列表里的视图和网关用的路由：插件表现在是什么样，外面（App、网关）看到的就是什么。

use std::sync::Arc;

use serde::Serialize;

use super::super::capabilities::WEB_STORAGE;
use super::super::entry::Entry;
use super::super::entry::Health;
use super::super::entry::PluginStatus;
use super::super::install_hosted::{effective_status, hosted_capability, provider_in};
use super::super::install_storage::storage_need;
use super::super::manifest::{LocalizedText, Manifest, PluginId, Sso};
use super::super::manifest_clients::ClientsHttp;
use super::super::manifest_static::content_security_policy;
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

/// 客户端入口（ADR-0021）网关转发要的东西：清单的 `clients.http` 原文和解析出的转发地址。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientsRoute {
    /// 清单的 `clients.http`：开关之外的检查（路径规则、auth、auth_failures）都按它来。
    pub http: ClientsHttp,
    /// 核心访问 `clients.http.service` 的地址；服务容器不在（插件没运行）时为空。
    pub upstream: Option<Endpoint>,
}

/// App 里显示的第三方客户端入口和网关认证信息。
#[derive(Debug, Clone)]
pub struct ClientsView {
    pub id: PluginId,
    pub label: String,
    pub route: ClientsRoute,
    pub switches: crate::clients::state::ClientsEntries,
    pub entry_label: Option<String>,
    pub tcp_names: Vec<String>,
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
    /// 托管插件（ADR-0080）：`upstream` 是提供方，网关还要带上插件 ID、加按清单算的响应头。
    pub hosted: Option<HostedRoute>,
    /// 声明了 `clients.http` 时的客户端入口（ADR-0021）；没声明时为空。
    pub clients: Option<ClientsRoute>,
}

/// 网关转发托管插件的请求要的东西。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostedRoute {
    /// 网关下发的 `Content-Security-Policy`（ADR-0080 第 4 节），盖掉提供方给的。
    pub csp: String,
    /// 提供方回 404 时改取的文件（相对站点根，不带开头的 `/`）。
    pub fallback: Option<String>,
    /// 清单写了 `web_storage`（ADR-0080 第 6 节）时 `/_wonton/data/` 转去哪；没写时为空，网关回 404。
    pub storage: Option<StorageRoute>,
}

/// 数据存储的转发目标。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StorageRoute {
    /// 提供方运行中时的地址；没装或没在运行时为空，网关回“没有就绪”。
    pub upstream: Option<Endpoint>,
}

/// 请求路径是否精确匹配本次路由清单登记的 OIDC 登录回调。
fn oidc_callback(entry: &Entry, path: &str) -> bool {
    entry
        .listed
        .as_ref()
        .and_then(|listed| listed.manifest.webui.as_ref())
        .filter(|webui| webui.sso == Sso::Oidc)
        .and_then(|webui| webui.oidc.as_ref())
        .is_some_and(|client| client.redirect_paths.iter().any(|p| p == path))
}

/// 清单声明了 `clients.http` 就给路由；转发地址用部署时解析的那个，清单变了就当没解析出来，
/// 等重新部署（ADR-0021 的开关关着不在这里看：开关在插件记录里，入口侧自己查）。
fn clients_route(manifest: Option<&Manifest>, entry: &Entry) -> Option<ClientsRoute> {
    let http = manifest?.clients.as_ref()?.http.clone()?;
    Some(ClientsRoute {
        upstream: entry
            .clients
            .as_ref()
            .filter(|resolved| resolved.http == http)
            .filter(|_| {
                matches!(
                    entry.status,
                    PluginStatus::Running | PluginStatus::Degraded { .. }
                ) && entry.serving.contains(&http.service)
            })
            .and_then(|resolved| resolved.upstream.clone()),
        http,
    })
}

impl PluginManager {
    pub fn route(&self, slug: &str, path: &str) -> Option<Route> {
        let langs = self.owner_languages();
        let plugins = self.plugins.read().expect("插件表锁中毒");
        let (id, entry) = plugins.iter().find(|(id, _)| id.slug() == slug)?;
        let manifest = entry.listed.as_ref().map(|listed| &listed.manifest);
        if let Some(capability) = manifest.and_then(hosted_capability) {
            let spec = manifest
                .and_then(|m| m.static_web.clone())
                .unwrap_or_default();
            return Some(Route {
                id: id.clone(),
                status: effective_status(&plugins, entry, &langs),
                upstream: provider_in(&plugins, capability, &langs).and_then(|p| p.endpoint),
                web_serving: false,
                cross_origin_isolated: manifest
                    .and_then(|m| m.webui.as_ref())
                    .is_some_and(|webui| webui.cross_origin_isolated),
                proxy: entry.proxy.clone(),
                oidc_callback: oidc_callback(entry, path),
                hosted: Some(HostedRoute {
                    csp: content_security_policy(&spec),
                    fallback: spec.fallback,
                    storage: manifest.and_then(storage_need).map(|_| StorageRoute {
                        upstream: provider_in(&plugins, WEB_STORAGE.name, &langs)
                            .and_then(|p| p.endpoint),
                    }),
                }),
                clients: clients_route(manifest, entry),
            });
        }
        Some(Route {
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
            oidc_callback: oidc_callback(entry, path),
            hosted: None,
            clients: clients_route(manifest, entry),
        })
    }

    pub fn client_views(&self) -> Result<Vec<ClientsView>, super::super::store::StoreError> {
        let langs = self.owner_languages();
        let plugins = self.plugins.read().expect("插件表锁中毒");
        let mut views = Vec::new();
        for (id, entry) in plugins.iter() {
            let Some(listed) = entry.listed.as_ref() else {
                continue;
            };
            let Some(clients) = listed.manifest.clients.as_ref() else {
                continue;
            };
            let Some(http) = clients.http.as_ref() else {
                continue;
            };
            let record = self.store.load(id)?;
            views.push(ClientsView {
                id: id.clone(),
                label: langs.pick_owned(&listed.manifest.title),
                route: ClientsRoute {
                    upstream: entry
                        .clients
                        .as_ref()
                        .and_then(|route| route.upstream.clone()),
                    http: http.clone(),
                },
                switches: record
                    .as_ref()
                    .map_or_else(Default::default, |record| record.clients_entries),
                entry_label: record
                    .as_ref()
                    .and_then(|record| record.entry_label.clone()),
                tcp_names: clients
                    .tcp
                    .as_deref()
                    .unwrap_or_default()
                    .iter()
                    .map(|tcp| tcp.name.clone())
                    .collect(),
            });
        }
        Ok(views)
    }

    pub fn client_view(
        &self,
        id: &PluginId,
    ) -> Result<Option<ClientsView>, super::super::store::StoreError> {
        Ok(self.client_views()?.into_iter().find(|view| view.id == *id))
    }

    pub fn gateway_auth_headers(
        &self,
        id: &PluginId,
        plugin_admin: bool,
    ) -> Result<Vec<(hyper::header::HeaderName, hyper::header::HeaderValue)>, String> {
        let plugins = self.plugins.read().expect("插件表锁中毒");
        let entry = plugins.get(id).ok_or_else(|| "插件未安装".to_owned())?;
        let listed = entry
            .listed
            .as_ref()
            .ok_or_else(|| "插件清单不可用".to_owned())?;
        let gateway = listed
            .manifest
            .clients
            .as_ref()
            .and_then(|clients| clients.http.as_ref())
            .and_then(|http| http.auth.gateway.as_ref())
            .ok_or_else(|| "插件不是 gateway 密码入口".to_owned())?;
        let secrets = self
            .store
            .secrets()
            .load(id)
            .map_err(|error| error.to_string())?
            .unwrap_or_default();
        let generated = super::super::secrets::generated_values(
            &listed.manifest,
            &secrets,
            &self.owner_languages(),
        )
        .map_err(|error| error.to_string())?;
        let owner = self.cfg.profile.owner();
        let rendered = super::super::proxy_auth::render_gateway_headers(
            &gateway.headers,
            &owner,
            plugin_admin,
            &generated,
            &self.owner_languages(),
        )
        .map_err(|error| error.to_string())?;
        rendered
            .into_iter()
            .map(|(name, value)| {
                let value = hyper::header::HeaderValue::from_str(&value)
                    .map_err(|error| error.to_string())?;
                Ok((name, value))
            })
            .collect()
    }

    /// 第三方客户端入口验证 Basic 用的用户名（ADR-0021「接口」）：这批密码都挂在主人名下。
    pub(crate) fn owner_username(&self) -> String {
        self.cfg.profile.owner().username
    }

    pub(in crate::plugin) fn view(&self, id: &PluginId, entry: &Entry) -> PluginView {
        PluginView {
            id: id.clone(),
            version: entry.version.clone(),
            title: entry.title.clone(),
            // 托管插件刚装上时提供方的状态在插件表里，调用方可能正拿着写锁：这里给记录里的状态，列表和网关现看。
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
