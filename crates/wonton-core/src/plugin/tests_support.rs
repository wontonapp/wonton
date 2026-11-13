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

    /// 测试用：登记一个托管能力的提供方（ADR-0080），网关把托管插件的请求转到 `upstream`；为空时是没在运行。
    #[cfg(test)]
    pub(crate) fn register_hosted_provider_for_test(
        &self,
        manifest: Manifest,
        capability: &str,
        upstream: Option<String>,
    ) {
        let id = manifest.id.clone();
        let status = if upstream.is_some() {
            PluginStatus::Running
        } else {
            PluginStatus::Stopped
        };
        let mut entry = Entry::new(Listed::of_manifest(manifest), status, None);
        if let Some(upstream) = upstream {
            entry
                .hosted
                .insert(capability.to_owned(), Endpoint::Tcp(upstream));
        }
        self.plugins
            .write()
            .expect("插件表锁中毒")
            .insert(id, entry);
    }

    /// 测试用：给登记过的插件设客户端入口的开关和标签。开关和标签都存插件记录（ADR-0021），
    /// 这里走真的记录写入，再唤醒插件表，入口侧的缓存和连接守卫照生产路径刷新。
    #[cfg(test)]
    pub(crate) fn clients_entry_for_test(
        &self,
        id: &PluginId,
        label: &str,
        lan: bool,
        public: bool,
    ) {
        let mut record = self
            .store
            .load(id)
            .ok()
            .flatten()
            .unwrap_or_else(|| minimal_record(id));
        record.entry_label = Some(label.to_owned());
        record.clients_entries = crate::clients::state::ClientsEntries { lan, public };
        self.store.save(&record).expect("写测试记录");
        self.changed();
    }

    /// 测试用：给登记过的插件设 `clients.http` 的转发地址。生产路径在部署时经 Docker 解析
    /// （[`Self::clients_upstream`]），测试直接指到本地的假上游。
    #[cfg(test)]
    pub(crate) fn clients_route_for_test(
        &self,
        id: &PluginId,
        http: super::manifest_clients::ClientsHttp,
        upstream: Option<String>,
    ) {
        let mut plugins = self.plugins.write().expect("插件表锁中毒");
        let entry = plugins.get_mut(id).expect("先 register_for_test");
        entry.serving.insert(http.service.clone());
        entry.clients = Some(super::manager::ClientsRoute {
            http,
            upstream: upstream.map(Endpoint::Tcp),
        });
        drop(plugins);
        self.changed();
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

    #[cfg(test)]
    pub(crate) fn clients_status_for_test(&self, id: &PluginId, status: PluginStatus) {
        let mut plugins = self.plugins.write().expect("插件表锁中毒");
        plugins.get_mut(id).expect("先 register_for_test").status = status;
        drop(plugins);
        self.changed();
    }
}

/// 测试用的最小安装记录：只够 `store.save` 落盘和入口侧按标签查到开关。
#[cfg(test)]
fn minimal_record(id: &PluginId) -> InstalledRecord {
    InstalledRecord {
        lan_webui: false,
        relays: Vec::new(),
        firewalled: false,
        proxied: false,
        id: id.clone(),
        version: "1.0.0".into(),
        source: "services: {}".into(),
        files: Default::default(),
        granted_permissions: vec![],
        enabled_permissions: Vec::new(),
        limits: Default::default(),
        oidc_client_secret: None,
        installed_at: 0,
        entry_label: None,
        clients_entries: Default::default(),
        setup_done_ever: false,
        origin: None,
        written_by: None,
        ask_every_update: false,
        family: false,
        settings: Default::default(),
        settings_changed_ms: None,
        static_files: Default::default(),
        hosted: Vec::new(),
        gpu_vendor: None,
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
        )
        .chain(
            manifest
                .provides
                .iter()
                .flat_map(|provides| provides.capabilities.values().map(|c| &c.service)),
        );
    entry.serving.extend(services.cloned());
}
