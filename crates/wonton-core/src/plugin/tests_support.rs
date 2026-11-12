//! 测试用的登记入口：不经 Docker 把插件放进管理器，供网关测试指向本地的假上游。

use std::sync::Arc;

use super::listed::Listed;
use super::manager::{Entry, PluginManager, PluginStatus};
use super::manifest::{Manifest, PluginId};
use super::proxy_auth::ProxyAuth;
use super::upstreams::{Endpoint, McpTarget};

impl PluginManager {
    /// 测试用：不经 Docker 直接登记一个插件，供网关测试指向本地的假上游。
    #[cfg(test)]
    pub(crate) fn register_for_test(
        &self,
        manifest: Manifest,
        status: PluginStatus,
        upstream: Option<String>,
    ) {
        let upstream = upstream.map(Endpoint::Tcp);
        let id = manifest.id.clone();
        let entry = Entry::new(Listed::of_manifest(manifest), status, upstream);
        self.plugins
            .write()
            .expect("插件表锁中毒")
            .insert(id, entry);
    }

    /// 测试用：给登记过的插件设上 MCP 服务的地址。
    #[cfg(test)]
    pub(crate) fn mcp_for_test(&self, id: &PluginId, url: String) {
        let mut plugins = self.plugins.write().expect("插件表锁中毒");
        plugins.get_mut(id).expect("先 register_for_test").mcp =
            Some(McpTarget { url, socket: None });
        drop(plugins);
        self.changed();
    }

    /// 测试用：给登记过的插件设上网关代登录的凭据。
    #[cfg(test)]
    pub(crate) fn proxy_for_test(&self, id: &PluginId, auth: ProxyAuth) {
        let mut plugins = self.plugins.write().expect("插件表锁中毒");
        plugins.get_mut(id).expect("先 register_for_test").proxy = Some(Arc::new(auth));
    }
}
