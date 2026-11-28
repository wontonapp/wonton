//! 容器地址解析：服务在这个网络里的地址，和隧道端口对应的套接字地址。

use std::net::{IpAddr, SocketAddr};

use super::super::manager::{InstallError, PluginManager};
use super::super::manifest::PluginId;
use super::super::package::PluginPackage;
use wonton_protocol::PortProtocol;
use wonton_tunnel::PortResolveError;

impl PluginManager {
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
