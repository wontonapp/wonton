//! 插件详情页：页面打开的是哪一页，以及核心说这一页接下来还要不要显示。

use serde::{Deserialize, Serialize};

use super::WidgetCapability;
use super::{
    ConsumerCapabilitySetting, InstalledPlugin, LanWebuiOffer, MissingCapability, PermissionLine,
    PluginAuthor, PluginIcon, PluginImage, PluginKind, PluginLimits, PluginRuntime, PluginScope,
    PluginProblem, PluginTrust, RequirementLine, SecretInputState, SecurityTier, hint,
};
use crate::{CardLine, PluginShares};

/// 插件的详情页：`GET /_wonton/api/store/plugin?source=…`（来源最新的版本）或
/// `GET /_wonton/api/plugins/{id}`（装着的版本）。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct PluginDetail {
    /// 安装来源（不带版本）；上传的和预装的插件为空。
    pub source: Option<String>,
    pub id: String,
    pub title: String,
    pub summary: String,
    /// 详细介绍：纯文本，空行分段；作者没写时为空。
    pub about: Option<String>,
    pub icon: Option<PluginIcon>,
    /// 截图，按作者给的顺序；取不到的图片显示占位。
    pub screenshots: Vec<PluginImage>,
    pub tags: Vec<String>,
    pub kind: PluginKind,
    #[serde(default)]
    pub runtime: PluginRuntime,
    /// 核心按清单算出的安全档位（ADR-0080）；清单读不出来或旧核心不给时为空。
    #[serde(default)]
    pub security_tier: Option<SecurityTier>,
    /// `runtime` 是 [`PluginRuntime::Link`] 时的网页地址，壳原样交给系统的 App 内浏览器；其他形态为空。
    #[serde(default)]
    pub link_url: Option<String>,
    /// 装卸权限是 Allow 时可见的安装范围。
    #[serde(default)]
    pub scope: Option<PluginScope>,
    /// 主人可修改的角色开放范围；其他人只收到风险类别，不收到角色列表。
    #[serde(default)]
    pub audience: Option<crate::PluginAudienceView>,
    /// 这个人能否直接安装；要管理员批时为 false。
    #[serde(default)]
    pub installable: bool,
    #[serde(default)]
    pub install_gate: crate::Gate,
    /// 详情说的版本。
    pub version: String,
    pub trust: PluginTrust,
    /// 核心能核实的提供方，如 `github.com/owner/repo`；上传的和预装的插件为空。清单里的 `author` 是作者自己写的，要分开显示。
    pub provider: Option<String>,
    /// 经第三方索引装的插件是索引的主机，如 `example.com`：装哪个提交由它决定。
    pub via_index: Option<String>,
    pub author: Option<PluginAuthor>,
    /// 应用的主页。
    pub homepage: Option<String>,
    /// 插件仓库的网页，如 `https://github.com/owner/repo`。
    pub repository: Option<String>,
    /// 插件里那个应用的许可证（SPDX 表达式）。
    #[serde(default)]
    #[cfg_attr(feature = "uniffi", uniffi(default = None))]
    pub license: Option<String>,
    /// 许可证全文的地址（HTTPS），非 SPDX 许可证（`LicenseRef-`）用它给出处。
    #[serde(default)]
    #[cfg_attr(feature = "uniffi", uniffi(default = None))]
    pub license_url: Option<String>,
    /// GitHub 星数；不是来自 GitHub 的插件为空。
    pub stars: Option<u32>,
    /// 这个版本的更新说明。
    pub release_notes: Option<String>,
    /// 安装时要授予的权限。
    pub permissions: Vec<PermissionLine>,
    /// 这个版本的网页能当 Matrix widget 在房间里做什么（ADR-0096）；不是 widget 时为空。
    #[serde(default)]
    pub widget_capabilities: Vec<WidgetCapability>,
    /// 对宿主的要求和这台主机满不满足。
    pub requirements: Vec<RequirementLine>,
    /// 主人要填的密钥和填没填（ADR-0064），按清单里的顺序；装着的版本才列，`input` 项才在里面。没有值。
    #[serde(default)]
    pub secret_inputs: Vec<SecretInputState>,
    /// 装好后有网页（小程序）。
    pub has_web: bool,
    /// 这个版本的清单带设置（ADR-0077）：详情页显示设置入口，读 `GET …/settings`。
    #[serde(default)]
    pub has_settings: bool,
    /// 装着时是它的状态；没装时为空。
    pub installed: Option<InstalledPlugin>,
    /// 装不了的原因；能装时为空。
    pub blocked: Option<String>,
    /// 要主人知道的情况，如本机防火墙可能挡住了局域网设备；按顺序显示，`emphasis` 的醒目显示。
    #[serde(default)]
    pub notices: Vec<CardLine>,
    /// 装着的插件缺了某个平台能力、在用它自己的设置时的轻提示（如没接模型网关），类别见
    /// [`hint::PluginHintCategory`]；不是错误，状态照常，一页可以有多条。没有可提示的情况（没装、
    /// 能力都接上了）时是空列表。
    #[serde(default)]
    pub hints: Vec<hint::PluginHint>,
    /// 装着的插件现在的问题，壳按类别提示主人下一步（目前是选中的设备不在了，ADR-0036）；状态照常，没有时为空。
    #[serde(default)]
    pub problems: Vec<PluginProblem>,
    /// 缺的 `requires` 能力（ADR-0053）：`providers` 列索引里能提供它的插件；没有的装不上。
    #[serde(default)]
    pub missing_requires: Vec<MissingCapability>,
    /// 缺的 `wants` 能力（ADR-0053）：照装，少点能力。
    #[serde(default)]
    pub missing_wants: Vec<MissingCapability>,
    /// 使用方给每种能力单独指定的提供方（ADR-0053）；`selected` 为空表示跟全局默认走。
    #[serde(default)]
    pub capability_providers: Vec<ConsumerCapabilitySetting>,
    /// 这个版本有要主人打开开关才开的局域网端口时是开关的情况；`enabled` 是当前值，没装时是 `false`。
    pub lan_webui: Option<LanWebuiOffer>,
    /// 这个插件的 Bot 已获授权存文件的路径，如 `music/周杰伦`（ADR-0027）。
    #[serde(default)]
    pub save_grants: Vec<String>,
    /// 主人给它定的资源上限；没装时为空。
    #[serde(default)]
    pub limits: Option<PluginLimits>,
    /// 开关“每次都问”：Agent 写的插件是当前值，打开后这个 Agent 每次更新都发卡片；别的插件为空。
    #[serde(default)]
    pub ask_every_update: Option<bool>,
    /// 主人可管理的分享；没权限、没装或不支持分享接口时为空，不伪装成零人。
    #[serde(default)]
    pub shares: Option<PluginShares>,
}

/// 详情页打开的是哪一页。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PluginDetailRoute {
    /// 商店里这个来源最新的版本。
    Store { source: String },
    /// 主屏上装着的这个插件。
    Installed { id: String },
}

/// 详情页接下来要显示什么：核心判断插件还在不在，壳只照做。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "type", rename_all = "snake_case")]
#[expect(
    clippy::large_enum_variant,
    reason = "serialized by value across FFI, so variant sizes do not matter; UniFFI record fields do not support Box"
)]
pub enum PluginDetailOutcome {
    /// 插件还在，显示这一份详情。
    Detail { detail: PluginDetail },
    /// 插件已经卸掉，这一页没有东西可看了，壳关掉它。
    Close,
}
