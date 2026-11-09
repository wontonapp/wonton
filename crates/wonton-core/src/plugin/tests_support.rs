//! 测试用的登记入口：不经 Docker 把插件放进管理器，供网关测试指向本地的假上游。

use std::sync::Arc;

use super::listed::Listed;
use super::manager::{Entry, PluginManager, PluginStatus};
use super::manifest::{Manifest, PluginId};
use super::proxy_auth::ProxyAuth;
use super::secrets::Secrets;
use super::store::InstalledRecord;
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
        let mut entry = Entry::new(Listed::of_manifest(manifest.clone()), status, upstream);
        // 不经 Docker 没有容器可看：清单里的入口一律当成能用。
        serve_declared(&mut entry, &manifest);
        self.plugins
            .write()
            .expect("插件表锁中毒")
            .insert(id, entry);
    }

    /// 测试用：读插件的安装记录。
    #[cfg(test)]
    pub(crate) fn record_for_test(&self, id: &PluginId) -> Option<InstalledRecord> {
        self.store.load(id).ok().flatten()
    }

    /// 测试用：给登记过的插件塞密钥值，供主人查看接口的测试。
    #[cfg(test)]
    pub(crate) fn secrets_for_test(&self, id: &PluginId, values: &[(&str, &str)]) {
        let secrets: Secrets = values
            .iter()
            .map(|(name, value)| ((*name).to_owned(), value.as_bytes().to_vec()))
            .collect();
        self.store.secrets().save(id, &secrets).expect("写测试密钥");
    }

    /// 测试用：给登记过的插件设上 MCP 服务的地址。
    #[cfg(test)]
    pub(crate) fn mcp_for_test(&self, id: &PluginId, url: String) {
        let mut plugins = self.plugins.write().expect("插件表锁中毒");
        let entry = plugins.get_mut(id).expect("先 register_for_test");
        if let Some(service) = entry
            .listed
            .as_ref()
            .and_then(|listed| listed.manifest.mcp.as_ref())
            .map(|mcp| mcp.service.clone())
        {
            entry.serving.insert(service);
        }
        entry.mcp = Some(McpTarget {
            url,
            socket: None,
            headers: Default::default(),
        });
        drop(plugins);
        self.changed();
    }

    /// 测试用：给登记过的插件设上网关代登录的凭据。
    #[cfg(test)]
    pub(crate) fn proxy_for_test(&self, id: &PluginId, auth: ProxyAuth) {
        let mut plugins = self.plugins.write().expect("插件表锁中毒");
        plugins.get_mut(id).expect("先 register_for_test").proxy = Some(Arc::new(auth));
    }

    /// 测试用：直接给出巡检看到的“能用的服务”，代替看一眼容器。
    #[cfg(test)]
    pub(crate) fn serving_for_test(&self, id: &PluginId, services: &[&str]) {
        let mut plugins = self.plugins.write().expect("插件表锁中毒");
        plugins.get_mut(id).expect("先 register_for_test").serving = services
            .iter()
            .map(|service| (*service).to_owned())
            .collect();
        drop(plugins);
        self.changed();
    }
}

/// 清单里当入口的服务（网页、MCP、模型网关）一律当成能用。
#[cfg(test)]
fn serve_declared(entry: &mut Entry, manifest: &Manifest) {
    let services = manifest
        .webui
        .as_ref()
        .map(|webui| &webui.service)
        .into_iter()
        .chain(manifest.mcp.as_ref().map(|mcp| &mcp.service))
        .chain(
            manifest
                .provides
                .as_ref()
                .and_then(|provides| provides.models.as_ref())
                .map(|models| &models.service),
        );
    entry.serving.extend(services.cloned());
}
