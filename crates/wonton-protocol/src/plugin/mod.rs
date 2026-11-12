//! 插件商店和已安装的插件（ADR-0017 的“接口：App 和核心”）。
//!
//! 核心的管理接口（`/_wonton/api/` 下）收发这里的类型；客户端核心把它们放进 [`PluginsViewModel`] 交给壳。文字由核心按
//! 请求的 `Accept-Language` 挑好语言，壳原样显示。图片都经核心处理成位图（[`PluginImage`]），壳不直接取插件给的图片。
//!
//! 核心可能比 App 新：核心多给的字段被忽略；枚举遇到不认识的取值时落到 `Other` 或 `Unknown`，不让整份列表解析失败。

mod capability_deps;
pub mod hint;
mod owner_secrets;
mod permission;
mod secret_input;
#[path = "task.rs"]
mod task;
mod tool_policy;
pub use capability_deps::{
    BlockedCategory, CapabilityDefaultChoice, CapabilityProvider, CapabilityProviderRef,
    CapabilityProviderSettings, CapabilitySetting, ConsumerCapabilityChoice,
    ConsumerCapabilitySetting, InUseError, InstallTogether, MissingCapability, UninstallPreview,
    UninstallRequest,
};
use serde::{Deserialize, Serialize};

pub use owner_secrets::{OwnerPluginSecret, OwnerPluginSecretValue, OwnerPluginSecrets};
pub use permission::{
    PermissionLine, PermissionOpened, PermissionRisk, PermissionUpdate, PluginDevice,
};
pub use secret_input::{
    SecretInput, SecretInputState, SecretUpdate, SecretValues, secret_input_path,
};
pub use task::{PluginOperation, PluginTask, PluginTaskState, UpgradeResult};
pub use tool_policy::{
    AddedTools, BotSwitch, OpenWindow, ToolAccess, ToolPolicyEntry, ToolPolicyView,
};

use crate::{
    CardLine, CoreUpgradeViewModel, EgressViewModel, LanPort, PluginChat, PluginTab,
    RegistryMirrorViewModel, TabDisplay,
};

/// 插件从哪里来，决定商店、确认页和已安装列表上的标签。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum PluginTrust {
    /// 官方索引里审核过的版本。
    Official,
    /// 没审核过、来自 GitHub 或第三方索引：社区列表里的，或者主人添加的来源里的。
    Community,
    /// 主人上传的插件包，也没审核过。
    Local,
    /// 随核心预装的系统插件，不能卸载。
    System,
    /// 比这个 App 新的核心给的来源，当作未审核显示。
    #[serde(other)]
    Unknown,
}

/// 插件的种类（清单的 `kind`）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum PluginKind {
    App,
    Agent,
    Channel,
    Model,
    System,
    #[serde(other)]
    Other,
}

/// 核心处理过的一张图片：图标是 PNG，截图是 JPEG。用 `ClientCore::load_plugin_image` 取。
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct PluginImage {
    /// 核心上的地址（以 `/_wonton/api/images/` 开头）。同一个地址的内容不变，可以当缓存的键。
    pub path: String,
}

/// 插件的图标：正方形、满幅，壳裁成圆角方形显示。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct PluginIcon {
    /// 192×192，列表和主屏用。
    pub small: PluginImage,
    /// 512×512，详情页和确认页用。
    pub large: PluginImage,
}

/// 插件包的作者，不一定是插件里那个应用的作者。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct PluginAuthor {
    pub name: String,
    pub url: Option<String>,
}

/// 写这个插件的 Agent（ADR-0033）。Agent 的名字核实不了代码位置，所以不算 `provider`。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct PluginWriter {
    /// Agent 插件的 ID。
    pub plugin_id: String,
    /// Agent 的名字，如 `Hermes`，显示成“你的 Agent · Hermes”。
    pub title: String,
}

/// `GET /_wonton/api/plugins` 的响应。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct InstalledPlugins {
    /// 系统插件在前，其余按安装的先后。
    pub plugins: Vec<InstalledPlugin>,
    /// 检查新版本没成功的原因（如连不上 GitHub），这时升级提示可能不全；刚检查过时为空。
    pub upgrade_check: Option<StoreProblem>,
}

/// 一个已安装的插件：主屏和已安装列表里的一项。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct InstalledPlugin {
    pub id: String,
    pub title: String,
    /// 一句话说明（清单的 `description`）。
    pub summary: String,
    pub version: String,
    /// 插件内容变化时随之变化，仅用于比较相等。
    pub revision: String,
    pub kind: PluginKind,
    /// 没有图标时为空，壳画占位图标。
    pub icon: Option<PluginIcon>,
    pub trust: PluginTrust,
    /// 安装来源，如 `github:owner/repo@v1.2.0`；上传的和预装的为空。
    pub source: Option<String>,
    /// 核心能核实的提供方，如 `github.com/owner/repo`；上传的和预装的插件为空。清单里的 `author` 是作者自己写的，要分开显示。
    pub provider: Option<String>,
    /// 经第三方索引装的插件是索引的主机，如 `example.com`：装哪个提交由它决定。
    pub via_index: Option<String>,
    /// Agent 写的插件是写它的 Agent。主人用文件升级或卸载后为空，这个插件不再归 Agent。
    #[serde(default)]
    pub written_by: Option<PluginWriter>,
    pub status: PluginState,
    /// 插件网页的地址，点开就是小程序；没有网页时为空。
    pub web_url: Option<String>,
    /// 清单 `webui` 的打开方式，没声明 `tabs` 的插件固定后用它。
    #[serde(default)]
    pub web_display: TabDisplay,
    /// 清单 `webui` 声明了页面要跨源隔离：Android 壳只对这些插件的网页放宽 WebView 的隔离限制。
    #[serde(default)]
    pub cross_origin_isolated: bool,
    /// 清单声明了 `mcp`：Bot 能调它的工具，网页标题栏上才有“我来操作”。
    #[serde(default)]
    pub declares_tools: bool,
    /// 插件声明的底栏页面。
    #[serde(default)]
    pub tabs: Vec<PluginTab>,
    /// 插件的 Bot 在聊天页上的声明（ADR-0060）；没有 Bot 的插件为空。
    #[serde(default)]
    pub chat: Option<PluginChat>,
    /// 这台设备的人在这个插件里的 Bot（Matrix 用户 ID）：Agent 插件点开就是和它的私聊。
    pub bot_user_id: Option<String>,
    /// 主人开放给家人了（ADR-0045）。家人的设备只拿到开放了的插件。
    #[serde(default)]
    pub family: bool,
    /// 能升级到的版本；没有新版本时为空。
    pub upgrade: Option<UpgradeOffer>,
    /// 主人能卸载它；系统插件不能。
    pub removable: bool,
    /// 要主人知道的情况，如“这个版本的 tag 被作者改到了别的提交”；按顺序显示，`emphasis` 的醒目显示。
    pub notices: Vec<CardLine>,
    /// 现在开着的最高级可选权限（ADR-0057），如 `docker.socket`：列表里据此带醒目标志，没有时为空。
    #[serde(default)]
    pub enabled_root_permissions: Vec<String>,
    /// 正在安装或升级时做到了哪一步；其余状态和还没开始部署时为空。
    #[serde(default)]
    pub install_progress: Option<InstallProgress>,
}

/// 插件的状态。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum PluginState {
    Installing,
    Running {
        health: PluginHealth,
    },
    /// 所有常驻服务的容器都已退出或尚未启动，不包括一次性服务。
    Stopped,
    Upgrading,
    /// 异常：部分容器未运行、健康检查没过、和清单不符等；`reason` 是核心写的原因。
    Degraded {
        reason: String,
    },
    Uninstalling,
    /// 比这个 App 新的核心给的状态。
    #[serde(other)]
    Unknown,
}

/// 运行中的插件最近一次健康检查的结果。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum PluginHealth {
    /// 每个长期运行的服务都有健康检查，而且都通过了。
    Healthy,
    /// 只知道在运行：有服务没有健康检查，或者核心启动后还没检查完。
    #[serde(other)]
    Unknown,
}

/// 一次安装或升级做到了哪一步，带这一步的进度。步骤由家里的服务器报，App 按 `stage` 挑文字。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct InstallProgress {
    pub stage: InstallStage,
    /// 这一步完成了多少，0 到 100；算不出来时为空，壳只显示阶段。
    pub percent: Option<u8>,
}

/// 安装或升级的阶段。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum InstallStage {
    /// 拉镜像。
    PullingImages,
    /// 建网络和卷、准备数据目录、代 Bot 登录。
    Preparing,
    /// 启动容器。
    Starting,
    /// 容器起来了，等检查通过、Bot 上线。
    WaitingForBot,
    /// 比这个 App 新的核心给的阶段。
    #[serde(other)]
    Other,
}

/// 能升级到的版本。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct UpgradeOffer {
    pub version: String,
    /// 新版本的更新说明；作者没写时为空。
    pub release_notes: Option<String>,
    /// 新版本申请的权限和装着的不一样，升级时要重新确认全部权限。
    pub permissions_changed: bool,
}

/// `GET /_wonton/api/store` 的响应，按这个顺序显示：官方、主人添加的来源、社区。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct StoreListing {
    /// 官方索引里审核过的插件，按索引里的顺序。
    pub official: StoreSection,
    /// 主人添加的来源，一个来源一段，按添加的先后。
    pub added: Vec<AddedSource>,
    /// GitHub 上的社区插件（未审核），按星数从多到少。
    pub community: StoreSection,
}

/// 插件压缩包的大小上限：核心不收更大的，壳也不把更大的读进内存。
pub const MAX_PLUGIN_ARCHIVE_BYTES: u64 = 16 << 20;

/// 主人添加的一个来源（ADR-0031）：`POST /_wonton/api/store/sources` 的响应，也是商店里的一段。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct AddedSource {
    /// 规范后的地址，如 `github:owner/repo` 或 `https://example.com/index.json`：删除时原样带回。
    pub address: String,
    pub kind: SourceKind,
    /// 这个来源是谁，如 `github.com/owner/repo` 或 `example.com`。
    pub provider: String,
    /// 索引自己写的名字；仓库和没写名字的索引为空。
    pub name: Option<String>,
    pub section: StoreSection,
}

/// 添加的来源是什么。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    /// 一个插件仓库。
    Repo,
    /// 一个第三方索引。
    Index,
    /// 比这个 App 新的核心给的种类。
    #[serde(other)]
    Other,
}

/// `POST /_wonton/api/store/sources` 的请求体。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct AddSourceRequest {
    /// 主人填的地址，原样交给核心：仓库地址、`owner/repo`、`github:` 来源，或者索引的 https 地址。
    pub address: String,
}

/// 商店的一部分列表，带着它是什么时候的、有没有问题。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct StoreSection {
    pub plugins: Vec<StoreItem>,
    /// 列表是什么时候从来源取到的（Unix 毫秒）；本地目录里的索引，或者从没取到过时为空。
    pub fetched_at_ms: Option<u64>,
    /// 列表可能不是最新的、或者不完整时的原因；刚取到的完整列表为空。
    pub problem: Option<StoreProblem>,
    /// 来源里有、但清单读不懂所以没列出的插件。
    #[serde(default)]
    pub unreadable: Vec<StoreUnreadable>,
}

/// 清单读不懂的插件。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct StoreUnreadable {
    /// 来源，如 `github:owner/repo`。
    pub source: String,
    pub reason: String,
}

/// 商店列表的问题。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct StoreProblem {
    pub reason: StoreFailure,
    /// 技术细节，给日志和排查用。给用户看的文字由 App 按 `reason` 写，列表的时间看 `StoreSection.fetched_at_ms`。
    pub details: String,
}

/// 商店列表的问题的原因。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum StoreFailure {
    /// 家里的服务器连不上来源（GitHub），给的是上次取到的列表。
    Unreachable,
    /// 连上了，但来源在时限内没有答完，给的是上次取到的列表。
    Timeout,
    /// GitHub 限速了，过几分钟再试。
    RateLimited,
    /// 官方索引的内容读不懂。
    InvalidIndex,
    /// 官方索引的格式比核心新，要升级核心。
    IndexTooNew,
    /// 有的仓库没读到，列表不完整。
    Incomplete,
    /// 主人关掉了这部分（社区插件）。
    Disabled,
    /// 来源的其他问题，如索引文件不见了或者太大。
    Upstream,
    /// 比这个 App 新的核心给的原因。
    #[serde(other)]
    Other,
}

/// 商店里的一个插件。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct StoreItem {
    /// 安装来源，如 `github:owner/repo` 或 `index+https://example.com/index.json#app.example`；详情和预览都用它，壳不解析。
    pub source: String,
    pub id: String,
    pub title: String,
    pub summary: String,
    pub icon: Option<PluginIcon>,
    pub tags: Vec<String>,
    pub kind: PluginKind,
    /// 来源最新的版本；官方插件是索引里的版本。
    pub version: String,
    pub trust: PluginTrust,
    /// 核心能核实的提供方，如 `github.com/owner/repo`；上传的和预装的插件为空。清单里的 `author` 是作者自己写的，要分开显示。
    pub provider: Option<String>,
    /// 经第三方索引装的插件是索引的主机，如 `example.com`：装哪个提交由它决定。
    pub via_index: Option<String>,
    pub author: Option<PluginAuthor>,
    /// GitHub 星数；官方插件为空。
    pub stars: Option<u32>,
    /// 装着的版本；没装时为空。
    pub installed_version: Option<String>,
    /// 申请了高权限，商店里标出来。
    pub high_permissions: bool,
    /// 装不了的原因（宿主架构不符、需要核心不支持的权限、规范版本比核心新）；能装时为空。
    pub blocked: Option<String>,
}

/// 主人给插件定的资源上限，对它的每个服务生效（ADR-0037）。也是 `PUT /_wonton/api/plugins/{id}/limits` 的请求体。
/// 为空的项没调过，用 compose 写的或者默认值；请求里写空就恢复默认。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct PluginLimits {
    /// 内存上限，字节。
    #[serde(default)]
    pub memory: Option<u64>,
    /// CPU 上限，百分比：`150` 是 1.5 个核。默认不限。
    #[serde(default)]
    pub cpu_percent: Option<u32>,
}

/// 这台主机满不满足一项要求。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum RequirementStatus {
    Met,
    /// 不满足，装不了，`blocked` 写着原因。
    Unmet,
    /// 可选的要求不满足：照常装，少点能力（如没有显卡，退回 CPU）。
    Degraded,
    /// 查不出来，或者比这个 App 新的状态。
    #[serde(other)]
    Unknown,
}

/// 插件对宿主的一项要求。文字由核心写好，壳照着显示。
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct RequirementLine {
    /// 如“需要 2G 内存”。
    pub text: String,
    pub status: RequirementStatus,
    /// 不满足或查不出来时缺什么，如“这台主机只有 1.5 GB 内存”。
    pub problem: Option<String>,
}

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
    pub license: Option<String>,
    /// GitHub 星数；不是来自 GitHub 的插件为空。
    pub stars: Option<u32>,
    /// 这个版本的更新说明。
    pub release_notes: Option<String>,
    /// 安装时要授予的权限。
    pub permissions: Vec<PermissionLine>,
    /// 对宿主的要求和这台主机满不满足。
    pub requirements: Vec<RequirementLine>,
    /// 主人要填的密钥和填没填（ADR-0064），按清单里的顺序；装着的版本才列，`input` 项才在里面。没有值。
    #[serde(default)]
    pub secret_inputs: Vec<SecretInputState>,
    /// 装好后有网页（小程序）。
    pub has_web: bool,
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
    pub shares: Option<crate::PluginShares>,
}

/// `PUT /_wonton/api/plugins/{id}/ask-every-update` 的请求体，响应是更新后的 [`PluginDetail`]。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct AskEveryUpdate {
    pub enabled: bool,
}

/// 开关 `lan_webui`：网页的局域网端口默认不开，主人看过风险后可以打开（ADR-0023 的“网页的局域网端口”）。
/// 改开关时核心重建开端口的容器，数据不动。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct LanWebuiOffer {
    /// 要开关打开才开的端口。
    pub ports: Vec<LanPort>,
    /// 核心写的风险说明，放在开关旁边。
    pub risk: String,
    /// 开关现在的值：新装时是 `false`；升级时是装着的版本的值。
    pub enabled: bool,
    /// 第三方客户端的连接信息（清单写了 `lan.client`，开关开着时才有）。
    #[serde(default)]
    pub client: Option<LanClient>,
    /// 连接信息取不到的原因。
    #[serde(default)]
    pub client_problem: Option<String>,
}

/// 第三方客户端（比如音乐播放器）在局域网上连这个插件用的地址和账号。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct LanClient {
    pub addresses: Vec<String>,
    pub username: String,
    pub password: String,
}

/// `POST /_wonton/api/plugins/preview` 和 `POST /_wonton/api/plugins/{id}/upgrade/preview` 的请求体（JSON）：按来源预览。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct InstallSource {
    /// 商店给的来源，或者 `github:<owner>/<repo>`，可以带 `@<tag>` 或 `@<40 位提交号>`；不带时是最新的正式版本。
    pub source: String,
}

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
    pub id: String,
    pub title: String,
    pub summary: String,
    pub version: String,
    pub kind: PluginKind,
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
    /// 要授予的全部权限；主人确认的就是这些。
    pub permissions: Vec<PermissionLine>,
    /// 要主人填的密钥（ADR-0064）；值不在预览里，跟着安装、升级请求上行。升级预览只列新增的和还没填的。
    #[serde(default)]
    pub secret_inputs: Vec<SecretInput>,
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
    /// 主人确认的权限：每项必需权限都要在，少一项装不上；可以带上可选项（老客户端会这么做），但它们不算打开；
    /// 多出预览里没有的项装不上。
    pub grant: Vec<String>,
    /// 主人在预览里打开的可选项（[`PermissionLine::optional`] 的 `permission`），没带就是全关。
    #[serde(default)]
    pub enabled_permissions: Vec<String>,
    /// 主人填的密钥值（[`SecretValues`]），键是预览里 [`SecretInput::name`]。
    #[serde(default)]
    pub secrets: SecretValues,
    /// 主人选择的设备 ID；没有设备选择时为空。
    #[serde(default)]
    pub devices: Vec<String>,
    /// 打开开关 `lan_webui`。预览没有 [`LanWebuiOffer`] 时不起作用。
    #[serde(default)]
    pub lan_webui: bool,
    /// 要一起装的提供方（ADR-0053），带安装来源（和预览的 `providers[].source` 一致）。
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
    /// 同 [`InstallRequest::lan_webui`]。
    #[serde(default)]
    pub lan_webui: bool,
}

/// 插件页面的 ViewModel：已安装的插件、商店，以及安装、升级、卸载的进展。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct PluginsViewModel {
    /// 已安装的插件；还没取到过时为空，不代表没装插件。
    pub installed: Option<InstalledPlugins>,
    /// 商店；还没取过时为空。
    pub store: Option<StoreListing>,
    /// 正在从家里的服务器取已安装列表或商店。
    pub refreshing: bool,
    /// 最近一次取列表失败的原因，这时显示的是上次取到的列表；再取成功后清空。
    pub problem: Option<PluginProblem>,
    /// 安装、升级、卸载的进展，按发起的先后。同一个插件同时只有一个。
    pub tasks: Vec<PluginTask>,
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
}

/// 这台设备上的“我来操作”，同时只操作一个插件。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct PluginTakeover {
    pub id: String,
    pub state: TakeoverState,
}

/// “我来操作”走到哪一步了。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum TakeoverState {
    /// 请求发出去了，家里的服务器还没回：Bot 的调用还没有停住。
    Starting,
    /// 家里的服务器确认了：Bot 的敏感调用都先停住。
    Operating,
    /// 没开始成，Bot 的调用照常判定。
    NotStarted { problem: PluginProblem },
    /// 操作到一半断了：续不上约，或者插件停了、设备被吊销。Bot 的调用不再停住。
    Lost { problem: PluginProblem },
}

/// 家里服务器存数据的那块盘。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct StorageUsage {
    pub total_bytes: u64,
    pub free_bytes: u64,
}

/// 和插件有关的操作失败了。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct PluginProblem {
    pub reason: PluginFailure,
    /// 核心写的说明，壳原样显示。
    pub message: String,
    /// “服务器太旧”时要的最低服务器版本；别的类别和不知道时为空。
    #[serde(default)]
    pub min_version: Option<String>,
}

/// 失败的原因，壳据此提示主人下一步。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum PluginFailure {
    /// 连不上家里的服务器。
    TransportUnavailable,
    /// 家里的服务器不认这台设备，或者不让它做这件事。
    Unauthorized,
    /// 家里的服务器连不上插件的来源（如 GitHub）。
    SourceUnavailable,
    /// 来源里没有这个插件或版本，或者没装这个插件。
    NotFound,
    /// 插件包在预览之后变了：要重新预览、重新确认。
    Changed,
    /// 核心不支持：需要的权限、宿主架构、比核心新的规范版本等。
    Unsupported,
    /// 已经装了，或者正在装、正在升级、正在卸载。
    Conflict,
    /// 插件包不合法。
    Invalid,
    /// 选择的设备在这台主机上不可用。
    DeviceUnavailable,
    /// 家里的服务器选不出局域网地址，申请了 `network.lan` 的插件装不上：要主人在局域网设置里选网卡或地址。
    LanUnavailable,
    /// 插件要开的局域网端口被宿主上的别的容器或程序占着：要主人停掉占着它的那个再试。
    LanPortTaken,
    /// 家里的服务器正在升级，升级结束前不能动插件（ADR-0025）。
    CoreUpgrading,
    /// 主人配的代理连不上或者拒绝了，出网插件装不上：要主人在代理设置里改地址，或者修好代理（ADR-0023）。
    EgressProxyUnavailable,
    /// 插件要用的资料（如邮箱）还没填：要主人在设置里填好再试（ADR-0030）。
    OwnerProfileIncomplete,
    /// 家里的服务器上出错了：Docker、身份提供方、Matrix、模型网关没就绪，或者插件没通过健康检查。
    Upstream,
    /// App 和家里服务器的版本对不上，有一边要更新。
    Protocol,
    /// 服务器比 App 旧，还没有这个接口（[`crate::minimum_server_version`]）：升级服务器后可用。
    /// 服务器不旧却回 404 时不归这一类。
    ServerTooOld,
}

#[cfg(test)]
mod tests;
