//! 插件表的只读查询：列表和状态、清单里的权限与名字，给 App、网关和兄弟模块看。

use std::net::IpAddr;
use std::path::PathBuf;
use std::sync::Arc;

use super::super::image_worker::ImageWorker;
use super::super::manifest::{LocalizedText, ManifestExt, Permission, PluginId};
use super::super::mcp_server::{McpServer, ToolLimits};
use super::super::models::ProviderState;
use super::super::sources::Source;
use super::super::store::Store;
use super::{PluginManager, PluginStatus, PluginView};

impl PluginManager {
    /// 存数据的那块盘。查数据目录，不查 `apps_dir`：它要等装第一个插件时才建。
    pub fn storage_usage(&self) -> std::io::Result<wonton_protocol::StorageUsage> {
        let data_dir = &self.cfg.data_dir;
        let stats = rustix::fs::statvfs(data_dir)?;
        let block = stats.f_frsize.max(1);
        Ok(wonton_protocol::StorageUsage {
            total_bytes: stats.f_blocks.saturating_mul(block),
            free_bytes: stats.f_bavail.saturating_mul(block),
        })
    }
    /// 核心数据目录。
    pub fn data_dir(&self) -> PathBuf {
        self.cfg.data_dir.clone()
    }
    /// 声明了 `mcp`、MCP 服务现在能用的插件：运行中，或者异常但 MCP 服务自己好着。
    pub fn mcp_servers(&self) -> Vec<McpServer> {
        let plugins = self.plugins.read().expect("插件表锁中毒");
        plugins
            .iter()
            .filter(|(_, entry)| {
                matches!(
                    entry.status,
                    PluginStatus::Running | PluginStatus::Degraded { .. }
                )
            })
            .filter_map(|(id, entry)| {
                let target = entry.mcp.clone()?;
                let mcp = entry.listed.as_ref()?.manifest.mcp.as_ref()?;
                if !entry.serving.contains(&mcp.service) {
                    return None;
                }
                Some(McpServer {
                    id: id.clone(),
                    title: entry.title.clone(),
                    url: target.url,
                    socket: target.socket,
                    headers: target.headers,
                    tool_limits: ToolLimits::of(Some(mcp)),
                    tools: mcp.tools.clone(),
                })
            })
            .collect()
    }

    /// 这个插件的最高级权限：它的工具每次都以 root 身份执行，不能放宽（ADR-0036）。
    pub fn requires_root_confirmation(&self, id: &PluginId) -> bool {
        let plugins = self.plugins.read().expect("插件表锁中毒");
        plugins
            .get(id)
            .and_then(|entry| entry.listed.as_ref())
            .is_some_and(|listed| {
                listed.manifest.permissions.iter().any(|p| {
                    super::super::privileges::risk(p)
                        == Some(super::super::privileges::CapabilityRisk::Highest)
                })
            })
    }

    /// 插件装上的清单要了 `owner.identity`：它能看见人的身份，读家人名单和别人的语言都靠它。
    pub fn gets_owner_identity(&self, id: &PluginId) -> bool {
        self.plugins
            .read()
            .expect("插件表锁中毒")
            .get(id)
            .and_then(|entry| entry.listed.as_ref())
            .is_some_and(|listed| listed.manifest.gets_owner_identity())
    }

    /// 来源地址落在哪个已安装插件的网络里（见 [`super::super::sources`]）。
    pub fn source(&self, ip: IpAddr) -> Option<Source> {
        self.sources.find(ip)
    }

    /// 测试用：摘掉一个插件的来源，模拟它还没轮到恢复、来源没登记。
    #[cfg(test)]
    pub(crate) fn forget_source(&self, id: &PluginId) {
        self.sources.remove(id);
    }
    /// 模型网关现在的状态：没装、装了没在运行（或者网关服务不可用）、可用。
    pub(in crate::plugin) fn models_provider(&self) -> ProviderState {
        let plugins = self.plugins.read().expect("插件表锁中毒");
        let Some((id, entry)) = plugins.iter().find(|(_, e)| e.provides_models) else {
            return ProviderState::Absent;
        };
        match (&entry.status, &entry.models) {
            (PluginStatus::Running | PluginStatus::Degraded { .. }, Some(route))
                if entry.serving.contains(&route.spec.service) =>
            {
                ProviderState::Ready(Box::new(route.clone()))
            }
            _ => ProviderState::NotReady(id.clone()),
        }
    }
    /// 已装插件的清单里有没有这项权限（不看主人当时是否授予）。
    pub fn has_permission(&self, id: &PluginId, permission: &Permission) -> bool {
        self.plugins
            .read()
            .expect("插件表锁中毒")
            .get(id)
            .and_then(|entry| entry.listed.as_ref())
            .is_some_and(|listed| listed.manifest.permissions.contains(permission))
    }
    pub fn list(&self) -> Vec<PluginView> {
        let plugins = self.plugins.read().expect("插件表锁中毒");
        let failed = self.failed_installs.read().expect("安装结果锁中毒");
        plugins
            .iter()
            .map(|(id, entry)| self.view(id, entry))
            .chain(failed.iter().map(|(id, entry)| {
                let mut view = self.view(id, &entry.entry);
                view.url = None;
                view
            }))
            .collect()
    }

    /// 处理插件包里的图标和截图（校验插件包时用，`plugin check` 同一份代码）。
    pub fn images(&self) -> &Arc<ImageWorker> {
        self.gallery.images()
    }
    /// 插件记录和密钥库（卡片里要用密钥库判断哪一项已经填过）。
    pub(crate) fn store(&self) -> &Store {
        &self.store
    }

    pub(crate) fn is_installed(&self, id: &PluginId) -> bool {
        self.plugins.read().expect("插件表锁中毒").contains_key(id)
    }

    pub(in crate::plugin) fn status_of(&self, id: &PluginId) -> Option<PluginStatus> {
        let plugins = self.plugins.read().expect("插件表锁中毒");
        plugins
            .get(id)
            .map(|entry| entry.status.clone())
            .or_else(|| {
                self.failed_installs
                    .read()
                    .expect("安装结果锁中毒")
                    .get(id)
                    .map(|failed| failed.entry.status.clone())
            })
    }
    /// 插件的名字（多语言）。
    pub fn title(&self, id: &PluginId) -> Option<LocalizedText> {
        let plugins = self.plugins.read().expect("插件表锁中毒");
        plugins.get(id).map(|entry| entry.title.clone())
    }

    /// 一个 Bot 的显示名：清单 `bot.display_name`，没写时用插件标题。
    pub fn bot_name(&self, id: &PluginId, _bot: &str) -> Option<LocalizedText> {
        let plugins = self.plugins.read().expect("插件表锁中毒");
        let entry = plugins.get(id)?;
        entry
            .listed
            .as_ref()
            .and_then(|listed| listed.manifest.bot.as_ref())
            .and_then(|bot| bot.display_name.clone())
            .or_else(|| Some(entry.title.clone()))
    }
    /// 插件有没有网页：通知里带 `path` 的前提。
    pub fn has_webui(&self, id: &PluginId) -> bool {
        self.plugins
            .read()
            .expect("插件表锁中毒")
            .get(id)
            .is_some_and(|entry| entry.has_webui)
    }

    /// 是安装器预装的系统插件。
    pub fn is_system(&self, id: &PluginId) -> bool {
        self.plugins
            .read()
            .expect("插件表锁中毒")
            .get(id)
            .is_some_and(|entry| entry.system)
    }

    /// 随核心镜像附带、由核心自己维护的系统插件。从商店、来源或上传的同名 system 插件不算，
    /// 它们和普通插件一样能经接口升级、卸载（ADR-0108）。
    pub fn is_bundled_system(&self, id: &PluginId) -> bool {
        self.is_system(id) && self.bundled_system_record(id)
    }
    /// 记录或内存集合表明这个插件随核心带。界面层拿着 entry 时也用它，和四道闸门同一套判定。
    pub(in crate::plugin) fn bundled_system_record(&self, id: &PluginId) -> bool {
        if self.bundled.read().expect("插件表锁中毒").contains(id) {
            return true;
        }
        self.store
            .load(id)
            .ok()
            .flatten()
            .is_some_and(|record| record.bundled)
    }

    /// OIDC 客户端（ID 是插件的 `id`，Bot 的网页客户端按它的插件 ID 查）声明的管理员组和角色声明；
    /// 插件没登记时返回空值。
    pub fn oidc_identity_claims(
        &self,
        client_id: &str,
    ) -> (Vec<String>, Option<super::super::manifest::RoleClaim>) {
        let identity = match self.bot_web_oidc_identity(client_id) {
            Ok(identity) => identity,
            Err(error) => {
                tracing::warn!(%error, "Cannot read Bot OIDC identity");
                return (Vec::new(), None);
            }
        };
        let Ok(id) = PluginId::parse(
            identity
                .as_ref()
                .map_or(client_id, |bot| bot.plugin_id.as_str()),
        ) else {
            return (Vec::new(), None);
        };
        self.plugins
            .read()
            .expect("插件表锁中毒")
            .get(&id)
            .and_then(|entry| entry.listed.as_ref())
            .and_then(|listed| listed.manifest.webui.as_ref())
            .and_then(|webui| webui.oidc.as_ref())
            .map(|client| (client.admin_groups.clone(), client.role_claim.clone()))
            .unwrap_or_default()
    }
}
