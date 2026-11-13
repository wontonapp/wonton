//! 安装与升级的预览、确认请求；能力提供方复用同一套决定。

use serde::{Deserialize, Serialize};

use super::{
    BlockedCategory, BotSwitch, CapabilityProviderRef, InstallTogether, LanWebuiOffer,
    MissingCapability, PermissionLine, PluginAuthor, PluginDevice, PluginIcon, PluginKind,
    PluginRuntime, PluginScope, PluginTrust, PluginWriter, SecretInput, SecretValues, SecurityTier,
    SettingValues, SettingsForm, WidgetCapability,
};
use crate::CardLine;

/// 安装或升级前给主人确认的内容：预览接口的响应。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct InstallPreview {
    /// 解析后的来源，带着 tag 或提交，如 `github:owner/repo@v1.2.0`；上传的插件包是 `upload:sha256:…`，核心暂存 1 小时。
    /// 安装和升级请求原样带回。
    pub source: Option<String>,
    /// 来源解析到的提交（40 位十六进制）；上传的插件包为空。
    pub commit: Option<String>,
    /// 插件包摘要（`sha256:…`）：安装请求原样带回，插件包在预览后变了就装不上。
    pub digest: String,
    /// 不透明的安装计划：绑定本包摘要、已装提供方与全局默认、使用方—提供方—能力连接集合。
    /// [`InstallRequest`] 原样带回；核心重算不符时返回 409 `changed`，重新预览，不能自动授权新连接。
    /// 升级预览复用此类型，但 [`UpgradeRequest`] 不携带此字段，不适用这项计划绑定。
    #[serde(default)]
    pub plan: String,
    pub id: String,
    pub title: String,
    pub summary: String,
    pub version: String,
    pub kind: PluginKind,
    #[serde(default)]
    pub runtime: PluginRuntime,
    /// 核心按清单算出的安全档位（ADR-0080）；旧核心不给时为空。
    #[serde(default)]
    pub security_tier: Option<SecurityTier>,
    #[serde(default)]
    pub audience: Option<crate::PluginAudienceView>,
    #[serde(default)]
    pub install_gate: crate::Gate,
    /// 可供主人选择的 Bot；只有一个 Bot 时为空，壳不显示范围选择。
    #[serde(default)]
    pub scope_choices: Vec<crate::BotRef>,
    /// 清单声明了 Bot：装的时候聊天列表要给它占一行，装好后是和它的私聊。
    #[serde(default)]
    pub has_bot: bool,
    pub icon: Option<PluginIcon>,
    pub trust: PluginTrust,
    /// 核心能核实的提供方，如 `github.com/owner/repo`；上传的和预装的插件为空。清单里的 `author` 是作者自己写的，要分开显示。
    pub provider: Option<String>,
    /// 经第三方索引装的插件是索引的主机，如 `example.com`：装哪个提交由它决定。
    pub via_index: Option<String>,
    /// Agent 写的插件是写它的 Agent。
    #[serde(default)]
    pub written_by: Option<PluginWriter>,
    pub author: Option<PluginAuthor>,
    /// 应用的许可证原文（SPDX 表达式）；清单没写时为空。
    #[serde(default)]
    #[cfg_attr(feature = "uniffi", uniffi(default = None))]
    pub license: Option<String>,
    /// 许可证全文的地址（HTTPS），非 SPDX 许可证（`LicenseRef-`）用它给出处；清单没写时为空。
    #[serde(default)]
    #[cfg_attr(feature = "uniffi", uniffi(default = None))]
    pub license_url: Option<String>,
    /// 要授予的全部权限；主人确认的就是这些。
    pub permissions: Vec<PermissionLine>,
    /// 插件的网页能当 Matrix widget 在房间里做什么（ADR-0096）；不是 widget 时为空。按级别醒目显示，
    /// 升级时新申请的能力 `added` 为真。
    #[serde(default)]
    pub widget_capabilities: Vec<WidgetCapability>,
    /// 要主人填的密钥（ADR-0064）；值不在预览里，跟着安装、升级请求上行。升级预览只列新增的和还没填的。
    #[serde(default)]
    pub secret_inputs: Vec<SecretInput>,
    /// 要主人填的设置（ADR-0077）：按清单的 Schema 展开成表单，值跟安装、升级请求上行。没有设置时为空。
    /// 升级预览里带上装着版本的值；新 Schema 下不合法的项带 `problem`，改好才能确认。
    #[serde(default)]
    pub settings: Option<SettingsForm>,
    /// 确认前要看到的其他说明：未审核、镜像来自哪里、留着旧数据、降级、换来源、对宿主的要求等，按顺序显示，`emphasis` 的醒目显示。
    pub notices: Vec<CardLine>,
    /// 缺的 `requires` 能力（ADR-0053）：索引里有提供方的列出插件，主人确认一起装；没有的装不上。
    #[serde(default)]
    pub missing_requires: Vec<MissingCapability>,
    /// 缺的 `wants` 能力（ADR-0053）：照装，少点能力。
    #[serde(default)]
    pub missing_wants: Vec<MissingCapability>,
    /// 装的是提供方、而已装的插件正 `wants` 这个能力时（ADR-0053 的“高权限能力”），列出它们：不自动
    /// 接上，主人在安装请求里勾选的才接上；没勾的以后在它们的详情页里接。
    #[serde(default)]
    pub wanting_plugins: Vec<CapabilityProviderRef>,
    /// 升级或换版本时是装着的版本；新装时为空。
    pub upgrade_from: Option<String>,
    /// 这个版本的更新说明。
    pub release_notes: Option<String>,
    /// 装不了的原因；能装时为空，确认按钮这时不可点。
    pub blocked: Option<String>,
    /// 装不了的类别（ADR-0053）：App 按类别写文字，`blocked` 是细节。其他原因核心只写 `blocked`。
    #[serde(default)]
    #[cfg_attr(feature = "uniffi", uniffi(default = None))]
    pub blocked_category: Option<BlockedCategory>,
    /// 要先填资料才能装时的说明，确认按钮这时不可点。壳显示这句话和去填资料的按钮，填完重新预览。
    pub missing_profile: Option<String>,
    /// 可供主人选择、交给插件服务的宿主设备；不需要选择时为空。
    #[serde(default)]
    pub devices: Option<Vec<PluginDevice>>,
    /// 有要主人打开开关才开的局域网端口时是开关的情况；主人的选择放在请求的 `lan_webui` 字段带回。
    pub lan_webui: Option<LanWebuiOffer>,
    /// 升级时已经对这个插件始终允许的 Bot（ADR-0024）：新版本多出来的工具也算在内，主人在确认页可以关。新装时为空。
    pub always_allowed: Vec<BotSwitch>,
}

/// `POST /_wonton/api/plugins` 的请求体（JSON）：按来源安装。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct InstallRequest {
    /// 预览给的 `source`。
    pub source: String,
    /// 预览给的 `digest`；对不上时得到 409 `changed`。
    pub digest: String,
    /// 预览给的 `plan`，不解析、不重建。所有层的摘要、计划、输入与授权都核对后才能产生安装副作用。
    #[serde(default)]
    pub plan: String,
    /// 主人选择的范围；省略时按全局（[`PluginScope::All`]）安装。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<PluginScope>,
    /// 主人选择开放给哪些角色；省略时只保留“全部应用”的角色。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audience: Option<crate::PluginAudience>,
    /// 主人确认的权限：每项必需权限都要在，少一项装不上；可选项只有 enabled_permissions 打开。
    /// 多出预览里没有的项装不上。
    pub grant: Vec<String>,
    /// 主人在预览里打开的可选项（[`PermissionLine::optional`] 的 `permission`），没带就是全关。
    #[serde(default)]
    pub enabled_permissions: Vec<String>,
    /// 主人填的密钥值（[`SecretValues`]），键是预览里 [`SecretInput::name`]。
    #[serde(default)]
    pub secrets: SecretValues,
    /// 主人填的设置值（[`SettingValues`]），键是预览里 [`super::PluginSettingField::name`]。没带的项用默认值；必填项缺了
    /// 返回 400 `settings_invalid`。
    #[serde(default)]
    pub settings: SettingValues,
    /// 主人选择的设备 ID；没有设备选择时为空。
    #[serde(default)]
    pub devices: Vec<String>,
    /// 打开开关 `lan_webui`。预览没有 [`LanWebuiOffer`] 时不起作用。
    #[serde(default)]
    pub lan_webui: bool,
    /// 要一起装的提供方及其完整安装决定；只允许一层，内层不能再一起装或连接其他使用方。
    #[serde(default)]
    pub install_together: Vec<InstallTogether>,
    /// 要接上的已装使用方插件 ID（ADR-0053 的“高权限能力”）：装的是提供方、预览里列了
    /// `wanting_plugins`，主人勾选的才接上。
    #[serde(default)]
    pub connect_plugins: Vec<String>,
}

/// `POST /_wonton/api/plugins/{id}/upgrade` 的请求体（JSON）：换到升级预览给的版本。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct UpgradeRequest {
    /// 主人选择的设备 ID；没有设备选择时为空。
    #[serde(default)]
    pub devices: Vec<String>,
    /// 预览给的 `source`。主人没有主动选来源、只是升到提示的版本时为空。
    #[serde(default)]
    pub source: Option<String>,
    pub digest: String,
    /// 同 [`InstallRequest::grant`]。
    pub grant: Vec<String>,
    /// 同 [`InstallRequest::enabled_permissions`]。
    #[serde(default)]
    pub enabled_permissions: Vec<String>,
    /// 同 [`InstallRequest::secrets`]：升级只要新增的、还没填的项。
    #[serde(default)]
    pub secrets: SecretValues,
    /// 同 [`InstallRequest::settings`]：只带改了的项，没带的沿用装着版本的值。
    #[serde(default)]
    pub settings: SettingValues,
    /// 同 [`InstallRequest::lan_webui`]。
    #[serde(default)]
    pub lan_webui: bool,
}
