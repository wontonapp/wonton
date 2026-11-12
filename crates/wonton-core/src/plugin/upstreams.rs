//! 核心和插件之间的入口：核心在插件网络里的别名（插件经它们访问核心转发的服务），以及核心访问插件的地址
//! （网页、模型网关、MCP 服务）。宿主网络里的服务经 [`super::host_relay`] 的 unix 套接字访问。

use std::path::{Path, PathBuf};

use std::time::Duration;

use super::PluginId;
use super::host_relay::{HostRelay, Purpose};
use super::manager::{InstallError, PluginManager, PluginStatus};
use super::manifest::{Manifest, Permission, Sso};
use super::models::ModelsRoute;
use super::package::PluginPackage;
use super::proxy_auth::McpHeaders;
use super::secrets::Secrets;
use super::store::StoreError;
use crate::config::{PLUGIN_MATRIX_HOST, PLUGIN_MCP_HOST, PLUGIN_MODELS_HOST, PublicUrls};
use crate::http::HttpError;

/// MCP 服务没写 `path` 时的路径。
const DEFAULT_MCP_PATH: &str = "/mcp";

/// 网页探测一次的时限，和装完时的探测（[`super::health::wait_webui`]）一样。
const PROBE_TIMEOUT: Duration = Duration::from_secs(5);

/// 核心访问插件服务的地址：普通网络里是 `host:port`，宿主网络里是插件数据目录里的 unix 套接字。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Endpoint {
    Tcp(String),
    Unix(PathBuf),
}

impl Endpoint {
    pub fn unix(path: impl Into<PathBuf>) -> Self {
        Self::Unix(path.into())
    }

    /// 给 HTTP 客户端用的 URL。unix 套接字的主机名不参与解析，用 `localhost` 占位。
    pub fn url(&self, path: &str) -> String {
        match self {
            Self::Tcp(authority) => format!("http://{authority}{path}"),
            Self::Unix(_) => format!("http://localhost{path}"),
        }
    }

    pub fn socket(&self) -> Option<&Path> {
        match self {
            Self::Tcp(_) => None,
            Self::Unix(path) => Some(path),
        }
    }
}

/// 总线连一个插件的 MCP 服务要的东西：地址、解析好的请求头，宿主网络里还有套接字。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpTarget {
    pub url: String,
    pub socket: Option<PathBuf>,
    pub headers: McpHeaders,
}

/// 请求一次网页入口，返回状态码。unix 套接字走 reqwest，普通网络走共享的 HTTP 客户端。
pub(super) async fn status(
    http: &crate::http::HttpClient,
    endpoint: &Endpoint,
    path: &str,
    timeout: Duration,
) -> Result<u16, HttpError> {
    let Some(socket) = endpoint.socket() else {
        return http.get_status(&endpoint.url(path), timeout).await;
    };
    let client = reqwest::Client::builder()
        .unix_socket(socket.to_path_buf())
        .no_proxy()
        .build()
        .map_err(|e| HttpError::Connect(e.to_string()))?;
    let request = client.get(endpoint.url(path)).send();
    tokio::time::timeout(timeout, request)
        .await
        .map_err(|_| HttpError::Timeout(timeout))?
        .map(|response| response.status().as_u16())
        .map_err(|e| HttpError::Connect(e.to_string()))
}

pub(super) fn uses_sso(manifest: &Manifest) -> bool {
    manifest.webui.as_ref().is_some_and(|w| w.sso == Sso::Oidc)
}

/// 核心在插件网络里的别名：插件经它们访问核心代为转发的服务。SSO 插件经 `auth.<base>` 访问身份提供方，
/// Bot 经 `matrix.wonton.internal` 访问 Matrix 客户端接口，模型网关的使用方经 `models.wonton.internal`，
/// Agent 经 `mcp.wonton.internal` 访问 MCP 总线。
pub(super) fn core_aliases(public: &PublicUrls, manifest: &Manifest) -> Vec<String> {
    let mut aliases = Vec::new();
    if uses_sso(manifest) {
        aliases.push(public.auth_host());
    }
    let granted = |permission: Permission| manifest.permissions.contains(&permission);
    if granted(Permission::MatrixBot) {
        aliases.push(PLUGIN_MATRIX_HOST.to_owned());
    }
    if granted(Permission::Models) {
        aliases.push(PLUGIN_MODELS_HOST.to_owned());
    }
    if granted(Permission::Tools) {
        aliases.push(PLUGIN_MCP_HOST.to_owned());
    }
    aliases
}

/// [`PluginManager::resource_upstream`] 给不出地址的原因。
#[derive(Debug, thiserror::Error)]
pub enum ResourceUpstreamError {
    #[error("插件卸了或没在运行")]
    Unavailable,
    #[error("主机名不是这个插件 compose 里的服务名")]
    NotItsService,
    /// 读不了插件记录、解析不了插件包、Docker 出错：核心这边的问题，和插件是否在运行无关。
    #[error("{0}")]
    Internal(String),
}

impl PluginManager {
    /// 核心访问插件网页的地址。
    pub(super) async fn webui_upstream(
        &self,
        manifest: &Manifest,
        network: &str,
    ) -> Result<Option<Endpoint>, InstallError> {
        let Some(webui) = &manifest.webui else {
            return Ok(None);
        };
        let name = self
            .cfg
            .instance
            .container(&manifest.id.slug(), &webui.service);
        if let Some(endpoint) = self
            .host_relay_endpoint(manifest, Purpose::Webui, &webui.service)
            .await?
        {
            return Ok(Some(endpoint));
        }
        self.service_upstream(&name, webui.port, network)
            .await
            .map(Endpoint::Tcp)
            .map(Some)
    }

    /// 网页探测的状态码（ADR-0033 的 `wonton__plugin_status`）：请求一次网页入口，和装完时的探测
    /// （[`super::health::wait_webui`]）用同一个地址（核心在插件网络里看到的 `主机:端口` 加网页路径）
    /// 和每请求 5 秒的时限。没有网页、还没部署到那一步的为空。
    pub async fn webui_status(&self, id: &PluginId) -> Option<Result<u16, HttpError>> {
        let (endpoint, path) = {
            let plugins = self.plugins.read().expect("插件表锁中毒");
            let entry = plugins.get(id)?;
            (
                entry.upstream.clone()?,
                entry.listed.as_ref()?.manifest.webui.as_ref()?.path.clone(),
            )
        };
        Some(status(&self.http, &endpoint, &path, PROBE_TIMEOUT).await)
    }

    /// 是模型网关时：核心转发模型请求、执行凭据命令的目标。
    pub(super) async fn models_upstream(
        &self,
        manifest: &Manifest,
        network: &str,
    ) -> Result<Option<ModelsRoute>, InstallError> {
        let Some(spec) = manifest.provides.as_ref().and_then(|p| p.models.as_ref()) else {
            return Ok(None);
        };
        let slug = manifest.id.slug();
        let name = self.cfg.instance.container(&slug, &spec.service);
        let upstream = self.service_upstream(&name, spec.port, network).await?;
        Ok(Some(ModelsRoute {
            id: manifest.id.clone(),
            spec: spec.clone(),
            upstream,
            credentials_container: self
                .cfg
                .instance
                .container(&slug, spec.credentials_service()),
        }))
    }

    /// 声明了 `mcp` 时：MCP 总线连插件 MCP 服务的地址（`http://主机:端口/路径`），和按插件密钥解析好的
    /// 请求头。密钥还没有值、不是文本或者填不出合法的头值时安装失败：总线的头填不出来，宁可不上线。
    pub(super) async fn mcp_upstream(
        &self,
        manifest: &Manifest,
        network: &str,
        secrets: &Secrets,
    ) -> Result<Option<McpTarget>, InstallError> {
        let Some(mcp) = &manifest.mcp else {
            return Ok(None);
        };
        // 插件交的东西不对（密钥没写下、不是文本、填不出合法头值）：不是核心存储的错，按输入错误报。
        let headers = super::proxy_auth::resolve_mcp_headers(manifest, secrets).map_err(|e| {
            InstallError::Invalid(format!("插件 {} 的 MCP 请求头解析不了：{e}", manifest.id))
        })?;
        let path = mcp.path.as_deref().unwrap_or(DEFAULT_MCP_PATH);
        if let Some(endpoint) = self
            .host_relay_endpoint(manifest, Purpose::Mcp, &mcp.service)
            .await?
        {
            return Ok(Some(McpTarget {
                url: endpoint.url(path),
                socket: endpoint.socket().map(Path::to_path_buf),
                headers,
            }));
        }
        let name = self
            .cfg
            .instance
            .container(&manifest.id.slug(), &mcp.service);
        let upstream = self.service_upstream(&name, mcp.port, network).await?;
        Ok(Some(McpTarget {
            url: format!("http://{upstream}{path}"),
            socket: None,
            headers,
        }))
    }

    /// 服务是宿主网络、又该有中继时套接字地址；普通网络为空。
    async fn host_relay_endpoint(
        &self,
        manifest: &Manifest,
        purpose: Purpose,
        service: &str,
    ) -> Result<Option<Endpoint>, InstallError> {
        let name = self.cfg.instance.container(&manifest.id.slug(), service);
        let host = self
            .engine
            .inspect_container(&name)
            .await?
            .and_then(|info| info.host_config)
            .and_then(|config| config.network_mode)
            .is_some_and(|mode| mode == "host");
        if !host {
            return Ok(None);
        }
        let relay = HostRelay {
            purpose,
            service: service.to_owned(),
            port: 0,
        };
        let app_data = self.cfg.apps_dir.join(manifest.id.slug());
        Ok(Some(Endpoint::unix(super::host_relay::socket_path(
            &app_data, &relay,
        ))))
    }

    pub(crate) fn is_running(&self, id: &PluginId) -> bool {
        self.status_of(id) == Some(PluginStatus::Running)
    }

    /// 这个插件现在有没有这个共享文件夹的授权，读或写都算（ADR-0020 的“鉴权”）。卸了的插件没有。
    pub fn share_granted(&self, id: &PluginId, share: &str) -> Result<bool, StoreError> {
        let Some(record) = self.store.load(id)? else {
            return Ok(false);
        };
        Ok(record
            .granted_permissions
            .iter()
            .any(|p| matches!(p, Permission::StorageShare { name, .. } if name == share)))
    }

    /// 核心访问插件资源的内网地址（ADR-0020）。主机名必须是这个插件 compose 里的服务名；不同插件可能有同名的服务，所以换成这个
    /// 插件的容器的地址，不按主机名解析。
    pub async fn resource_upstream(
        &self,
        id: &PluginId,
        service: &str,
        port: u16,
    ) -> Result<String, ResourceUpstreamError> {
        if !self.is_running(id) {
            return Err(ResourceUpstreamError::Unavailable);
        }
        let internal = |why: String| ResourceUpstreamError::Internal(why);
        let record = self
            .store
            .load(id)
            .map_err(|e| internal(e.to_string()))?
            .ok_or(ResourceUpstreamError::Unavailable)?;
        let package = PluginPackage::with_files(&record.source, record.files)
            .map_err(|e| internal(e.to_string()))?;
        if !package.has_service(service) {
            return Err(ResourceUpstreamError::NotItsService);
        }
        let slug = id.slug();
        let name = self.cfg.instance.container(&slug, service);
        let network = self.cfg.instance.network(&slug);
        self.service_address(&name, port, &network)
            .await
            .map_err(|e| internal(e.to_string()))?
            .ok_or(ResourceUpstreamError::Unavailable)
    }

    /// 核心访问容器里一个端口的地址：核心在容器里时用容器名（同一网络内的 DNS），否则用容器 IP。
    pub(super) async fn service_upstream(
        &self,
        name: &str,
        port: u16,
        network: &str,
    ) -> Result<String, InstallError> {
        self.service_address(name, port, network)
            .await?
            .ok_or_else(|| {
                InstallError::Docker(format!(
                    "容器 {name} 不存在，或者在网络 {network} 中没有地址"
                ))
            })
    }

    /// 容器不在或者在这个网络里没有地址时为空。
    async fn service_address(
        &self,
        name: &str,
        port: u16,
        network: &str,
    ) -> Result<Option<String>, InstallError> {
        if self.cfg.core_container.is_some() {
            return Ok(Some(format!("{name}:{port}")));
        }
        let info = self.engine.inspect_container(name).await?;
        let ip = info
            .and_then(|info| info.network_settings)
            .and_then(|s| s.networks)
            .and_then(|mut n| n.remove(network))
            .and_then(|e| e.ip_address)
            .filter(|ip| !ip.is_empty());
        Ok(ip.map(|ip| format!("{ip}:{port}")))
    }
}
