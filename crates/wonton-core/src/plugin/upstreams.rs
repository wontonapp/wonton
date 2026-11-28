//! 核心和插件之间的入口：核心在插件网络里的别名（插件经它们访问核心转发的服务），以及核心访问插件的地址
//! （网页、模型网关、MCP 服务）。普通网络里连 [`names::service_host`] 的稳定域名（上游看到的 Host 就是它），
//! 宿主网络里的服务经 [`super::host_relay`] 的 unix 套接字访问。

mod aliases;
mod endpoint;
mod probe;
mod resource;
mod routes;
mod webui;

pub(super) use aliases::{core_aliases, uses_sso};
pub use endpoint::{Endpoint, McpTarget};
pub(super) use probe::{PROBE_TIMEOUT, probe, status};
pub use resource::ResourceUpstreamError;

use std::net::{IpAddr, SocketAddr};

use super::PluginId;
use super::health;
use super::health_gate::Wait;
use super::languages::Languages;
use super::manager::{InstallError, PluginManager};
use super::manifest::Manifest;
use super::models::ModelsRoute;
use super::names;
use super::package::PluginPackage;
use super::secrets::Secrets;
use crate::l10n;
use wonton_protocol::PortProtocol;
use wonton_tunnel::PortResolveError;

impl PluginManager {
    /// 安装、升级的容器起来之后：收存密钥，取网页、MCP 和模型网关的入口，声明了的入口取不到就是部署坏了，
    /// 最后等网页入口应一声。核对时容器本来就可能不在，所以入口函数给的是 `None`，这两条路要当错误。
    pub(super) async fn deploy_upstreams(
        &self,
        manifest: &Manifest,
        network: &str,
        secrets: &mut Secrets,
        wait: Wait,
        missing: super::secrets::CollectMissing,
        langs: &Languages,
    ) -> Result<(Option<Endpoint>, Option<ModelsRoute>, Option<McpTarget>), InstallError> {
        // 安装和升级到这里时一次性容器都已经退出（`start_containers` 等过）：它们该写下的密钥这会儿
        // 必须收到，不然 MCP 头填不出来。重启路径还在这一步收，没写下的下次再说。
        if super::secrets::collect(
            &self.engine,
            &self.cfg.instance,
            manifest,
            &self.bot_names(&manifest.id),
            secrets,
            missing,
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
            let container = self.webui_container(manifest, &webui.service);
            health::wait_webui(&self.http, upstream, &webui.path, &container, wait, langs).await?;
        }
        if super::bot_web_registry::has_bot_web(manifest) {
            for bot in self.store.bots().load(&manifest.id)?.bots.keys() {
                self.wait_bot_web(manifest, bot, wait).await?;
            }
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
    // 网页入口可以指向 Bot 服务，报的必须是默认 Bot 那份容器；其余入口校验保证不是 Bot 服务。
    let bots = super::bot_services::bot_services(manifest);
    let container = |service: &str| {
        if bots.contains(service) {
            instance.bot_container(&slug, service, super::bot_services::DEFAULT_BOT)
        } else {
            instance.container(&slug, service)
        }
    };
    let missing = |service: &str| {
        InstallError::Docker(l10n::text(
            langs,
            "plugin-install-upstream-container-missing",
            &[
                ("container", container(service).into()),
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
    use crate::plugin::languages::Languages;

    const BOT_WEB_MANIFEST: &str = "name: t
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
  permissions: [matrix.bot]
  bots:
    services: [web]
";

    #[test]
    fn a_missing_bot_web_container_is_named_with_its_bot() {
        let langs = Languages::from_header(Some("zh-Hans"));
        let manifest = PluginPackage::parse(BOT_WEB_MANIFEST, &langs)
            .unwrap()
            .manifest;
        let error = check_present(
            None,
            None,
            None,
            &manifest,
            &names::Instance::default(),
            "wonton-app-example-t",
            &langs,
        )
        .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("容器 wonton-app-example-t--web--default 不存在"),
            "{error}"
        );
    }
}
