use serde::{Deserialize, Serialize};

use super::store::{StoreConfirmation, UpdatablePlugin};
use super::{
    InstalledPlugins, OwnerSecretsView, PluginMediaGrant, PluginProblem, PluginTakeover,
    PluginTask, StorageUsage, StoreListing,
};
use crate::{CoreUpgradeViewModel, EgressViewModel, RegistryMirrorViewModel};

/// 插件页面的 ViewModel：已安装的插件、商店，以及安装、升级、卸载的进展。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct PluginsViewModel {
    /// 已安装的插件；还没取到过时为空，不代表没装插件。
    pub installed: Option<InstalledPlugins>,
    /// 商店；还没取过时为空。
    pub store: Option<StoreListing>,
    /// 侧边栏的内容。
    #[serde(default)]
    pub sidebar: crate::SidebarViewModel,
    /// 正在从家里的服务器取已安装列表或商店。
    pub refreshing: bool,
    /// 最近一次取列表失败的原因，这时显示的是上次取到的列表；再取成功后清空。
    pub problem: Option<PluginProblem>,
    /// 安装、升级、卸载的进展，按发起的先后。同一个插件同时只有一个。
    pub tasks: Vec<PluginTask>,
    /// 本设备发起的启动及其失败；成功后移除，不进入安装任务或磁盘缓存。
    #[serde(default)]
    pub starts: Vec<super::PluginStart>,
    /// 有新版本可升级的应用：商店页更新图标的角标、"全部更新"弹窗和侧边栏的红点都是它。顺序同已安装列表。
    #[serde(default)]
    pub updatable: Vec<UpdatablePlugin>,
    /// 主人点了一键装、一键升级（ [`PluginAction::InstallNow`](crate::PluginAction::InstallNow) 一类）、核心正在取预览的插件 ID：
    /// 行尾按钮转圈。取完要么开始（进 [`PluginsViewModel::tasks`]），要么要主人拿主意（进 [`PluginsViewModel::confirmation`]）。
    #[serde(default)]
    pub pending: Vec<String>,
    /// 核心取好预览、要主人拿主意的一键操作：壳照着打开确认页，主人在页上点安装或升级（[`PluginAction::InstallPlugin`](crate::PluginAction::InstallPlugin)、
    /// [`PluginAction::UpgradePlugin`](crate::PluginAction::UpgradePlugin)），或者关掉（[`PluginAction::DismissStoreConfirmation`](crate::PluginAction::DismissStoreConfirmation)）。
    #[serde(default)]
    pub confirmation: Option<StoreConfirmation>,
    /// 角色页（ADR-0088）：主人建、改、删角色，给成员和好友分配，设默认邀请角色。
    #[serde(default)]
    pub roles: crate::RolesViewModel,
    /// 出网代理的设置页。
    pub egress: EgressViewModel,
    /// Docker Hub 镜像加速的设置页。
    pub registry_mirror: RegistryMirrorViewModel,
    /// 核心的升级页。
    pub core_upgrade: CoreUpgradeViewModel,
    /// 家里服务器存数据的那块盘；还没取到、或者连不上时为空，这时不知道还剩多少。
    pub storage: Option<StorageUsage>,
    /// 连着家里的服务器却查不到磁盘空间的原因。
    pub storage_problem: Option<PluginProblem>,
    /// “我来操作”；没在操作时为空。结束了但不是主人交还的，留着告诉主人，等 `EndTakeover` 或者下一次 `StartTakeover`。
    pub takeover: Option<PluginTakeover>,
    /// 由核心给出的应用页入口；壳不按槽位种类自行判断，未知操作不显示。
    #[serde(default)]
    pub actions: Vec<crate::AppsAction>,
    /// 朋友分享的应用，来源独立于自家已装插件；未取到时为空。
    pub shared_apps: Option<crate::SharedAppsViewModel>,
    /// 这台设备上的插件端口映射；未取到或尚无映射时为空，不代表插件没有声明端口。
    #[serde(default)]
    pub port_mappings: Vec<crate::PluginPortMapping>,
    /// 仅在详情页主动展开时存在；收起、离页、后台和服务器切换时清除，不进入任何磁盘缓存。
    #[serde(default)]
    pub owner_secrets: Option<OwnerSecretsView>,
    /// 网页要摄像头/麦克风的决定，按插件 ID 记在本机；没问过的不在里面。按插件 ID 排序。
    #[serde(default)]
    pub media_grants: Vec<PluginMediaGrant>,
}
