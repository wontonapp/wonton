use serde::{Deserialize, Serialize};

use crate::{InstallPreview, PluginAudience};

/// 插件安装、管理、商店、代理、镜像和核心升级操作。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PluginAction {
    /// 重新取已安装的插件和它们能不能升级。
    RefreshPlugins,
    /// 展开插件生成给主人的密钥分组；每次重新取列表，不预取值。
    ListOwnerSecrets { plugin_id: String },
    /// 展开一项密钥，每次向服务器重新取值；只接受列表里的名字。
    ShowOwnerSecret { plugin_id: String, name: String },
    /// 收起一项并清除值；作废尚未完成的取值请求。
    HideOwnerSecret { plugin_id: String, name: String },
    /// 收起分组或离开详情页，清除列表、值和问题；晚到的请求不能恢复它们。
    DismissOwnerSecrets { plugin_id: String },
    /// 启动已停止的插件；已经运行时由核心幂等处理。
    StartPlugin { plugin_id: String },
    /// 在这台设备上开关插件的同号端口映射；未实现映射的客户端返回 `NotReady`，不改变开关或绑定端口。
    SetPortMapping { plugin_id: String, enabled: bool },
    /// 取商店列表；`force` 为真时家里的服务器不等缓存过期，直接去来源取。
    RefreshStore { force: bool },
    /// 主人在确认页上点了“安装”：按预览安装，授予预览里列出的全部权限。插件包在预览之后变了时失败
    /// （`PluginFailure::Changed`），要重新预览。主人打开了确认页上的开关时，把预览的 `lan_webui.enabled` 改成 `true` 再发。
    InstallPlugin {
        preview: InstallPreview,
        /// 主人在确认页上填的密钥。
        secrets: crate::SecretValues,
        /// 主人在确认页上填的设置。
        #[serde(default)]
        settings: crate::SettingValues,
    },
    /// 主人在升级确认页上点了“升级”，同 [`PluginAction::InstallPlugin`]。
    UpgradePlugin {
        preview: InstallPreview,
        secrets: crate::SecretValues,
        #[serde(default)]
        settings: crate::SettingValues,
    },
    /// 安装失败回滚后，使用核心保留的原始清单和主人已确认的输入重装，不重新弹确认页。
    RetryPluginInstall { id: String },
    /// 卸载。`delete_data` 为真时同时删除它的数据、密钥和 Bot 账号，不可恢复。
    UninstallPlugin {
        id: String,
        delete_data: bool,
        /// 一起卸的、再没有别的插件在用的提供方（ADR-0053）。
        #[serde(default)]
        #[cfg_attr(feature = "uniffi", uniffi(default = []))]
        providers: Vec<String>,
        /// 一起卸的依赖方插件（ADR-0053 的卸载被拦）：按顺序先卸它们，保留数据，再卸这个。
        #[serde(default)]
        #[cfg_attr(feature = "uniffi", uniffi(default = []))]
        dependents: Vec<String>,
    },
    /// 停止一个正在运行的插件；依赖它的插件会随之变成异常，照常显示原因。
    StopPlugin { plugin_id: String },
    /// 主人修改哪些角色能打开这个应用。
    SetPluginAudience {
        id: String,
        audience: PluginAudience,
    },
    /// 清掉一个做完的任务（成功或失败的提示）。
    DismissPluginTask { id: String },
    /// 在商店或详情页点“获取”：核心取预览后直接安装，或把需要主人确认的项放进 [`PluginsViewModel::confirmation`](crate::PluginsViewModel::confirmation)。
    /// 取预览失败和安装进展放在 [`PluginsViewModel::tasks`](crate::PluginsViewModel::tasks)。
    InstallNow { source: String },
    /// 主人点了"更新"，同 [`PluginAction::InstallNow`]，对装着的应用升级到提示的版本。
    UpgradeNow { plugin_id: String },
    /// 主人在商店页点了"全部更新"：逐个升级 [`PluginsViewModel::updatable`](crate::PluginsViewModel::updatable) 里的应用，
    /// 不用拿主意的直接开始；需要决定的留在批次行内，由主人打开确认页。
    UpgradeAll,
    /// 打开批次中一项需要主人决定的更新；不重新取预览。
    ReviewStoreUpdate { plugin_id: String },
    /// 清除已结束的批次结果，不取消更新，不清除无关任务。
    DismissStoreUpdates,
    /// 主人关掉了一键操作的确认页，没装也没升。还有排着队的就放下一条进来，没有就清空。
    DismissStoreConfirmation,
    /// 主人在插件网页的标题栏上点了“我来操作”。正在操作别的插件时，那个先交还。
    StartTakeover { id: String },
    /// 主人点了“交还”或者离开了这个网页。`id` 不是正在操作的插件时什么都不做，换页面时后一页的开始可能先到。
    /// 进后台不用发，核心自己交还。
    EndTakeover { id: String },
    /// 打开设置页：取设置。探测结果太旧时家里的服务器在后台再测，这里接着取几次，把新结果带回来。
    RefreshEgressProxy,
    /// 主人点了保存。`use_plugin` 选用已安装的代理插件，此时 `address` 必须为空；两者都空表示取消代理。
    /// `direct_cidrs` 用逗号或空白分隔。要重启插件时先放进 `egress.confirmation`，等 [`PluginAction::ConfirmEgressProxy`]。
    SaveEgressProxy {
        address: String,
        direct_cidrs: String,
        #[serde(default)]
        #[cfg_attr(feature = "uniffi", uniffi(default = false))]
        use_plugin: bool,
    },
    /// 主人确认了 `egress.confirmation` 列出的重启。
    ConfirmEgressProxy,
    /// 主人没有确认，什么都不改。
    DismissEgressConfirmation,
    /// 取镜像加速设置。
    RefreshRegistryMirror,
    /// 保存加速地址；空串表示关闭。
    SaveRegistryMirror { mirror: String },
    /// 测官方候选地址和当前使用地址，按顺序去重。
    TestRegistryMirrors,
    /// 取升级状态、查新版本。打开升级页时 `manual` 为假，主人点“检查”时为真。
    RefreshCoreUpgrade { manual: bool },
    /// 主人在确认页上点了“升级”。`digest` 是 [`CoreUpgradeOffer::digest`](crate::CoreUpgradeOffer::digest)。
    StartCoreUpgrade { digest: String },
    /// 请求取消，只在 [`CoreUpgradePhase::Pulling`](crate::CoreUpgradePhase::Pulling) 阶段有效。
    CancelCoreUpgrade,
    /// 这台设备不再提示这个版本。`digest` 是 [`CoreUpgradeOffer::digest`](crate::CoreUpgradeOffer::digest)。
    SkipCoreUpgrade { digest: String },
    /// 允许插件网页使用摄像头和麦克风；设为不允许会清除原决定，下次重新询问。
    SetPluginMediaPermission { plugin_id: String, allow: bool },
}
