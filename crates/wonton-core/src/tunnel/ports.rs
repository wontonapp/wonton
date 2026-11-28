use std::net::SocketAddr;
use std::sync::Arc;

/// 按插件安装记录、清单的 `tunnel.ports` 和运行状态解析端口。
pub struct PluginPortResolver {
    plugins: Arc<crate::plugin::PluginManager>,
}

impl std::fmt::Debug for PluginPortResolver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PluginPortResolver")
    }
}

impl PluginPortResolver {
    pub fn new(plugins: Arc<crate::plugin::PluginManager>) -> Self {
        Self { plugins }
    }
}

impl wonton_tunnel::PortResolver for PluginPortResolver {
    async fn resolve(
        &self,
        plugin: &str,
        port: u16,
        protocol: wonton_protocol::PortProtocol,
    ) -> Result<SocketAddr, wonton_tunnel::PortResolveError> {
        self.plugins.tunnel_port(plugin, port, protocol).await
    }
}
