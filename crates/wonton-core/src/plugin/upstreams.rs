//! 核心和插件之间的入口：核心在插件网络里的别名（插件经它们访问核心转发的服务），以及核心访问插件的地址
//! （网页、模型网关、MCP 服务）。普通网络里连 [`names::service_host`] 的稳定域名（上游看到的 Host 就是它），
//! 宿主网络里的服务经 [`super::host_relay`] 的 unix 套接字访问。

use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};

use std::time::Duration;

use tokio::time::Instant;

use super::PluginId;
use super::health;
use super::host_relay::{HostRelay, Purpose};
use super::languages::Languages;
use super::manager::{InstallError, PluginManager, PluginStatus};
use super::manifest::{Manifest, Permission, Sso};
use super::models::ModelsRoute;
use super::names;
use super::package::PluginPackage;
use super::proxy_auth::McpHeaders;
use super::secrets::Secrets;
use super::store::StoreError;
use crate::config::{
    PLUGIN_AUTH_HOST, PLUGIN_MATRIX_HOST, PLUGIN_MCP_HOST, PLUGIN_MODELS_HOST, PLUGIN_NOTIFY_HOST,
    PublicUrls,
};
use crate::http::HttpError;
use crate::l10n;
use wonton_protocol::PortProtocol;
use wonton_tunnel::PortResolveError;

/// MCP 服务没写 `path` 时的路径。
const DEFAULT_MCP_PATH: &str = "/mcp";

/// 网页探测一次的时限，和装完时的探测（[`super::health::wait_webui`]）、`plugins.health` 的详情接口
/// （[`super::health_probe`]）一样。
pub(super) const PROBE_TIMEOUT: Duration = Duration::from_secs(5);

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

/// 探测一次网页入口（`plugins.health` 的详情接口，规范“插件健康探测”）：请求带插件公开地址的 Host 头，
/// 像浏览器经网关访问那样，但不带任何主人身份。返回（结果，耗时）。期限 [`PROBE_TIMEOUT`] 由调用方传入。
pub(super) async fn probe(
    http: &crate::http::HttpClient,
    endpoint: &Endpoint,
    path: &str,
    host: &str,
    timeout: Duration,
) -> (Duration, Result<u16, HttpError>) {
    let started = std::time::Instant::now();
    let outcome = match endpoint.socket() {
        None => {
            let request = hyper::Request::get(endpoint.url(path))
                .header(hyper::header::HOST, host)
                .body(axum::body::Body::empty());
            match request {
                Ok(request) => match tokio::time::timeout(timeout, http.send(request)).await {
                    Ok(Ok(response)) => Ok(response.status().as_u16()),
                    Ok(Err(e)) => Err(e),
                    Err(_) => Err(HttpError::Timeout(timeout)),
                },
                Err(e) => Err(HttpError::BadUrl(e.to_string())),
            }
        }
        Some(socket) => {
            // 和 [`status`] 一样：unix 套接字的主机名不参与解析，reqwest 按套接字连。
            let client = match reqwest::Client::builder()
                .unix_socket(socket.to_path_buf())
                .no_proxy()
                .build()
            {
                Ok(client) => client,
                Err(e) => return (started.elapsed(), Err(HttpError::Connect(e.to_string()))),
            };
            let sent = client
                .get(endpoint.url(path))
                .header(reqwest::header::HOST, host)
                .send();
            match tokio::time::timeout(timeout, sent).await {
                Ok(Ok(response)) => Ok(response.status().as_u16()),
                Ok(Err(e)) => Err(HttpError::Connect(e.to_string())),
                Err(_) => Err(HttpError::Timeout(timeout)),
            }
        }
    };
    (started.elapsed(), outcome)
}

/// 核心在插件网络里的别名：插件经它们访问核心代为转发的服务。SSO 插件经 `auth.wonton.internal` 访问身份提供方，
/// 有网页的插件经自己的公网地址访问自己（ADR-0063），Bot 经 `matrix.wonton.internal` 访问 Matrix 客户端接口，
/// 模型网关的使用方经 `models.wonton.internal`，Agent 经 `mcp.wonton.internal` 访问 MCP 总线。
pub(super) fn core_aliases(public: &PublicUrls, manifest: &Manifest) -> Vec<String> {
    let mut aliases = Vec::new();
    if uses_sso(manifest) {
        aliases.push(PLUGIN_AUTH_HOST.to_owned());
    }
    if manifest.webui.is_some() {
        aliases.push(public.plugin_host(&manifest.id.slug()));
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
    if granted(Permission::PluginsHealth) {
        aliases.push(crate::config::PLUGIN_HEALTH_HOST.to_owned());
    }
    if granted(Permission::Notify) {
        aliases.push(PLUGIN_NOTIFY_HOST.to_owned());
    }
    for capability in manifest.requires.iter().chain(&manifest.wants) {
        if capability == super::capabilities::MODELS.name
            || super::capabilities::has_no_consumer(capability)
            || (super::capabilities::lookup(capability).is_none()
                && !super::capabilities::is_third_party(capability))
        {
            continue;
        }
        let alias = super::capabilities::alias(capability);
        if !aliases.contains(&alias) {
            aliases.push(alias);
        }
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
        let slug = manifest.id.slug();
        let name = self.cfg.instance.container(&slug, &webui.service);
        if let Some(endpoint) = self
            .host_relay_endpoint(manifest, Purpose::Webui, &webui.service, webui.port)
            .await?
        {
            return Ok(Some(endpoint));
        }
        self.service_address(
            &name,
            &names::service_host(&slug, &webui.service),
            webui.port,
            network,
        )
        .await
        .map(|address| address.map(Endpoint::Tcp))
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
        let Some(upstream) = self
            .service_address(
                &name,
                &names::service_host(&slug, &spec.service),
                spec.port,
                network,
            )
            .await?
        else {
            return Ok(None);
        };
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
        let langs = self.owner_languages();
        let headers =
            super::proxy_auth::resolve_mcp_headers(manifest, secrets, &langs).map_err(|e| {
                InstallError::Invalid(l10n::text(
                    &langs,
                    "plugin-install-upstream-mcp-headers",
                    &[
                        ("id", manifest.id.to_string().into()),
                        ("error", e.to_string().into()),
                    ],
                ))
            })?;
        let path = mcp.path.as_deref().unwrap_or(DEFAULT_MCP_PATH);
        if let Some(endpoint) = self
            .host_relay_endpoint(manifest, Purpose::Mcp, &mcp.service, mcp.port)
            .await?
        {
            return Ok(Some(McpTarget {
                url: endpoint.url(path),
                socket: endpoint.socket().map(Path::to_path_buf),
                headers,
            }));
        }
        let slug = manifest.id.slug();
        let name = self.cfg.instance.container(&slug, &mcp.service);
        let Some(upstream) = self
            .service_address(
                &name,
                &names::service_host(&slug, &mcp.service),
                mcp.port,
                network,
            )
            .await?
        else {
            return Ok(None);
        };
        Ok(Some(McpTarget {
            url: format!("http://{upstream}{path}"),
            socket: None,
            headers,
        }))
    }

    /// 服务是宿主网络、又该有中继时套接字地址；普通网络为空。
    pub(in crate::plugin) async fn host_relay_endpoint(
        &self,
        manifest: &Manifest,
        purpose: Purpose,
        service: &str,
        port: u16,
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
            port,
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
        let package =
            PluginPackage::with_files(&record.source, record.files, &self.owner_languages())
                .map_err(|e| internal(e.to_string()))?;
        if !package.has_service(service) {
            return Err(ResourceUpstreamError::NotItsService);
        }
        let slug = id.slug();
        let name = self.cfg.instance.container(&slug, service);
        let network = self.cfg.instance.network(&slug);
        self.service_address(&name, &names::service_host(&slug, service), port, &network)
            .await
            .map_err(|e| internal(e.to_string()))?
            .ok_or(ResourceUpstreamError::Unavailable)
    }

    /// 安装、升级的容器起来之后：收存密钥，取网页、MCP 和模型网关的入口，声明了的入口取不到就是部署坏了，
    /// 最后等网页入口应一声。核对时容器本来就可能不在，所以入口函数给的是 `None`，这两条路要当错误。
    pub(super) async fn deploy_upstreams(
        &self,
        manifest: &Manifest,
        network: &str,
        secrets: &mut Secrets,
        deadline: Instant,
        langs: &Languages,
    ) -> Result<(Option<Endpoint>, Option<ModelsRoute>, Option<McpTarget>), InstallError> {
        if super::secrets::collect(
            &self.engine,
            &self.cfg.instance,
            manifest,
            &self.bot_names(&manifest.id),
            secrets,
            langs,
        )
        .await?
        {
            self.store.secrets().save(&manifest.id, secrets)?;
        }
        let upstream = self.webui_upstream(manifest, network).await?;
        let models = self.models_upstream(manifest, network).await?;
        let mcp = self.mcp_upstream(manifest, network, secrets).await?;
        check_present(
            upstream.as_ref(),
            models.as_ref(),
            mcp.as_ref(),
            manifest,
            &self.cfg.instance,
            network,
            langs,
        )?;
        if let (Some(upstream), Some(webui)) = (&upstream, &manifest.webui) {
            health::wait_webui(&self.http, upstream, &webui.path, deadline, langs).await?;
        }
        Ok((upstream, models, mcp))
    }

    /// 容器不在或者在这个网络里没有地址时为空。
    pub(in crate::plugin) async fn service_address(
        &self,
        container: &str,
        host: &str,
        port: u16,
        network: &str,
    ) -> Result<Option<String>, InstallError> {
        if self.cfg.core_container.is_some() {
            return Ok(Some(format!("{host}:{port}")));
        }
        let info = self.engine.inspect_container(container).await?;
        let ip = info
            .and_then(|info| info.network_settings)
            .and_then(|s| s.networks)
            .and_then(|mut n| n.remove(network))
            .and_then(|e| e.ip_address)
            .filter(|ip| !ip.is_empty());
        Ok(ip.map(|ip| format!("{ip}:{port}")))
    }

    /// 隧道把已授权设备的 `Port`/`Udp` 请求解析到插件网络里的地址（ADR-0072）：只在清单声明过
    /// `(service, port, protocol)` 时放行，拒绝类别和“连不上”分开。
    pub(crate) async fn tunnel_port(
        &self,
        plugin: &str,
        port: u16,
        protocol: PortProtocol,
    ) -> Result<SocketAddr, PortResolveError> {
        use PortResolveError as Refused;

        if protocol == PortProtocol::Other {
            return Err(Refused::PortNotDeclared);
        }
        let id = PluginId::parse(plugin).map_err(|_| Refused::PluginNotInstalled)?;
        let internal = |why: String| {
            tracing::warn!(plugin, port, %why, "解析插件隧道端口失败");
            Refused::UpstreamUnavailable
        };
        let record = self
            .store
            .load(&id)
            .map_err(|e| internal(e.to_string()))?
            .ok_or(Refused::PluginNotInstalled)?;
        let package =
            PluginPackage::with_files(&record.source, record.files, &self.owner_languages())
                .map_err(|e| internal(e.to_string()))?;
        if package.manifest.id != id {
            return Err(internal("插件清单和安装记录的 ID 不符".to_owned()));
        }
        let service = package
            .manifest
            .tunnel
            .as_ref()
            .and_then(|tunnel| {
                tunnel
                    .ports
                    .iter()
                    .find(|spec| spec.port == port && spec.protocol == protocol)
            })
            .map(|spec| spec.service.as_str())
            .ok_or(Refused::PortNotDeclared)?;
        if !self.is_running(&id) {
            return Err(Refused::PluginNotRunning);
        }
        let slug = id.slug();
        let name = self.cfg.instance.container(&slug, service);
        let network = self.cfg.instance.network(&slug);
        // 即使核心在容器里也查这个插件网络的地址，不经 DNS 解析请求里的插件名。
        let endpoint = self
            .engine
            .inspect_container(&name)
            .await
            .map_err(|e| internal(e.to_string()))?
            .and_then(|info| info.network_settings)
            .and_then(|settings| settings.networks)
            .and_then(|mut networks| networks.remove(&network))
            .ok_or(Refused::UpstreamUnavailable)?;
        let address = endpoint
            .ip_address
            .as_deref()
            .filter(|ip| !ip.is_empty())
            .or_else(|| {
                endpoint
                    .global_ipv6_address
                    .as_deref()
                    .filter(|ip| !ip.is_empty())
            })
            .ok_or(Refused::UpstreamUnavailable)?;
        let ip = address
            .parse::<IpAddr>()
            .map_err(|e| internal(e.to_string()))?;
        Ok(SocketAddr::new(ip, port))
    }
}

/// 安装、升级刚确认过每个容器就绪，声明了的入口却取不到就是这次部署坏了。核对时容器本来就可能不在，
/// [`PluginManager::webui_upstream`] 这些给的是 `None`（原因在核对的 `problem` 里），这两条路要当错误。
pub(super) fn check_present(
    upstream: Option<&Endpoint>,
    models_route: Option<&ModelsRoute>,
    mcp_target: Option<&McpTarget>,
    manifest: &Manifest,
    instance: &names::Instance,
    network: &str,
    langs: &Languages,
) -> Result<(), InstallError> {
    let slug = manifest.id.slug();
    let missing = |service: &str| {
        InstallError::Docker(l10n::text(
            langs,
            "plugin-install-upstream-container-missing",
            &[
                ("container", instance.container(&slug, service).into()),
                ("network", network.into()),
            ],
        ))
    };
    if let Some(webui) = &manifest.webui
        && upstream.is_none()
    {
        return Err(missing(&webui.service));
    }
    if let Some(models) = manifest
        .provides
        .as_ref()
        .and_then(|provides| provides.models.as_ref())
        && models_route.is_none()
    {
        return Err(missing(&models.service));
    }
    if let Some(mcp) = &manifest.mcp
        && mcp_target.is_none()
    {
        return Err(missing(&mcp.service));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::package::PluginPackage;
    use super::*;
    use crate::config::{PLUGIN_HEALTH_HOST, PLUGIN_MODELS_HOST, PublicUrls};
    use crate::plugin::languages::Languages;

    const MANIFEST: &str = "name: t
services:
  web:
    image: example/t
x-wonton:
  spec: '0.4'
  id: app.example.t
  version: 0.1.0
  kind: app
  title: {zh: 测试, en: Test}
  description: {zh: 测试, en: Test}
  permissions: [models, tools, plugins.health, matrix.bot]
";

    const SSO_MANIFEST: &str = "name: t
services:
  web:
    image: example/t
x-wonton:
  spec: '0.4'
  id: app.example.t
  version: 0.1.0
  kind: app
  title: {zh: 测试, en: Test}
  description: {zh: 测试, en: Test}
  webui:
    service: web
    port: 3000
    sso: oidc
    oidc:
      redirect_paths: [/oidc/callback]
  permissions: []
";

    fn public() -> PublicUrls {
        PublicUrls {
            scheme: "http".into(),
            base_domain: "wonton.example".into(),
            port: 8080,
        }
    }

    #[test]
    fn health_alias_is_only_for_plugins_that_asked_for_it() {
        let manifest = PluginPackage::parse(MANIFEST, &Languages::from_header(Some("zh-Hans")))
            .unwrap()
            .manifest;
        let aliases = core_aliases(&public(), &manifest);
        assert!(aliases.contains(&PLUGIN_HEALTH_HOST.to_owned()));
        assert!(aliases.contains(&PLUGIN_MODELS_HOST.to_owned()));
        let mut without = PluginPackage::parse(MANIFEST, &Languages::from_header(Some("zh-Hans")))
            .unwrap()
            .manifest;
        without
            .permissions
            .retain(|p| *p != Permission::PluginsHealth);
        assert!(!core_aliases(&public(), &without).contains(&PLUGIN_HEALTH_HOST.to_owned()));
    }
    #[test]
    fn sso_plugins_reach_the_provider_by_its_internal_name_and_their_own_host() {
        let manifest = PluginPackage::parse(SSO_MANIFEST, &Languages::from_header(Some("zh-Hans")))
            .unwrap()
            .manifest;
        let aliases = core_aliases(&public(), &manifest);
        assert!(aliases.contains(&"auth.wonton.internal".to_owned()));
        assert!(!aliases.contains(&"auth.wonton.example".to_owned()));
        assert!(aliases.contains(&"app-example-t.wonton.example".to_owned()));
    }

    #[test]
    fn plugins_without_a_webui_get_no_alias_to_their_own_address() {
        let manifest = PluginPackage::parse(MANIFEST, &Languages::from_header(Some("zh-Hans")))
            .unwrap()
            .manifest;
        assert!(
            !core_aliases(&public(), &manifest)
                .contains(&"app-example-t.wonton.example".to_owned())
        );
    }

    #[test]
    fn dependency_aliases_only_include_known_or_valid_third_party_capabilities() {
        let mut manifest = PluginPackage::parse(MANIFEST, &Languages::from_header(None))
            .unwrap()
            .manifest;
        manifest.permissions.clear();
        manifest.requires = vec!["torrent_client".into(), "x-acme.ledger".into()];
        manifest.wants = vec![
            "jellyfin".into(),
            "torrent_client".into(),
            "models".into(),
            "egress_proxy".into(),
            "unknown".into(),
            "x-acme.invalid_name".into(),
        ];
        assert_eq!(
            core_aliases(&public(), &manifest),
            vec![
                "torrent_client.wonton.internal",
                "x-acme.ledger.wonton.internal",
                "jellyfin.wonton.internal",
            ],
        );
    }
}

#[cfg(test)]
mod probe_tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::UnixListener;

    /// 回 503 的套接字服务，核对 unix 套接字的探测也带 Host、拿得到状态码和耗时。
    #[tokio::test]
    async fn unix_socket_probe_carries_host_and_reports_status() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("web.sock");
        let listener = UnixListener::bind(&path).unwrap();
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut buf = [0u8; 2048];
            let n = stream.read(&mut buf).await.unwrap_or(0);
            let request = String::from_utf8_lossy(&buf[..n]);
            assert!(request.starts_with("GET / HTTP/1.1"));
            assert!(request.contains("host: app-example-t.wonton.example"));
            let _ = stream
                .write_all(b"HTTP/1.1 503 Service Unavailable\r\ncontent-length: 0\r\n\r\n")
                .await;
        });
        let http = crate::http::HttpClient::new();
        let (elapsed, outcome) = probe(
            &http,
            &Endpoint::unix(&path),
            "/",
            "app-example-t.wonton.example",
            PROBE_TIMEOUT,
        )
        .await;
        assert_eq!(outcome.unwrap(), 503);
        assert!(elapsed.as_millis() > 0);
    }
}
