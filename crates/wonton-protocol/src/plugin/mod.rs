//! 插件商店和已安装的插件（ADR-0017 的“接口：App 和核心”）。
//!
//! 核心的管理接口（`/_wonton/api/` 下）收发这里的类型；客户端核心把它们放进 [`PluginsViewModel`] 交给壳。文字由核心
//! 按请求的 `Accept-Language` 挑好语言，壳原样显示。图片都经核心处理成位图（[`PluginImage`]），壳不直接取插件给的图片。
//!
//! 核心可能比 App 新：核心多给的字段被忽略；枚举遇到不认识的取值时落到 `Other` 或 `Unknown`，不让整份列表解析失败。

use serde::{Deserialize, Serialize};

use crate::{CardLine, EgressViewModel, LanPort};

/// 插件从哪里来，决定商店、确认页和已安装列表上的标签。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum PluginTrust {
    /// 官方索引里审核过的版本。
    Official,
    /// GitHub 上的社区插件，云吞没有审核过。
    Community,
    /// 主人上传的插件包。
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

/// 商店分类（清单的 `category`，取值见插件规范的“商店信息”）。壳自己翻译分类名。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum PluginCategory {
    Ai,
    Agent,
    Photos,
    Media,
    Files,
    Documents,
    Productivity,
    Communication,
    Home,
    Network,
    Security,
    Developer,
    Downloads,
    Tools,
    System,
    /// 比这个 App 新的分类。
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

/// 要授予插件的一项权限。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct PermissionLine {
    /// 清单里的写法，如 `storage.share.photos:read`。
    pub permission: String,
    /// 给主人看的说明，如“读共享文件夹 photos”。
    pub text: String,
    /// 高权限（宿主网络、设备、GPU、Docker socket），醒目显示。
    pub high: bool,
    /// 核心支持这项权限；不支持时插件装不了。
    pub supported: bool,
    /// 升级时新申请的，标出来。
    pub added: bool,
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
    pub kind: PluginKind,
    pub category: Option<PluginCategory>,
    /// 没有图标时为空，壳画占位图标。
    pub icon: Option<PluginIcon>,
    pub trust: PluginTrust,
    /// 安装来源，如 `github:owner/repo@v1.2.0`；上传的和预装的为空。
    pub source: Option<String>,
    pub status: PluginState,
    /// 插件网页的地址，点开就是小程序；没有网页时为空。
    pub web_url: Option<String>,
    /// 插件的 Bot（Matrix 用户 ID）：Agent 插件点开就是和它的私聊。
    pub bot_user_id: Option<String>,
    /// 能升级到的版本；没有新版本时为空。
    pub upgrade: Option<UpgradeOffer>,
    /// 主人能卸载它；系统插件不能。
    pub removable: bool,
    /// 要主人知道的情况，如“这个版本的 tag 被作者改到了别的提交”；按顺序显示，`emphasis` 的醒目显示。
    pub notices: Vec<CardLine>,
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
    Upgrading,
    /// 异常：容器停了、健康检查没过、和清单不符等；`reason` 是核心写的原因。
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

/// `GET /_wonton/api/store` 的响应：官方插件和社区插件分开列。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct StoreListing {
    /// 官方索引里审核过的插件，按索引里的顺序。
    pub official: StoreSection,
    /// GitHub 上的社区插件（未审核），按星数从多到少。
    pub community: StoreSection,
}

/// 商店的一部分列表，带着它是什么时候的、有没有问题。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct StoreSection {
    pub plugins: Vec<StoreItem>,
    /// 列表是什么时候从来源取到的（Unix 毫秒）；用的是核心内置的那份、或者从没取到过时为空。
    pub fetched_at_ms: Option<u64>,
    /// 列表可能不是最新的、或者不完整时的原因；刚取到的完整列表为空。
    pub problem: Option<StoreProblem>,
}

/// 商店列表的问题。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct StoreProblem {
    pub reason: StoreFailure,
    /// 核心写的说明，如“连不上 GitHub，这是 3 小时前的列表”。
    pub message: String,
}

/// 商店列表的问题的原因。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum StoreFailure {
    /// 家里的服务器连不上来源（GitHub），给的是上次取到的或者核心内置的列表。
    Unreachable,
    /// GitHub 限速了，过几分钟再试。
    RateLimited,
    /// 有的仓库没读到，列表不完整。
    Incomplete,
    /// 主人关掉了这部分（社区插件）。
    Disabled,
    /// 来源给了核心读不懂的内容，如格式比核心新的索引。
    Upstream,
    /// 比这个 App 新的核心给的原因。
    #[serde(other)]
    Other,
}

/// 商店里的一个插件。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct StoreItem {
    /// 安装来源（不带版本），如 `github:owner/repo`；详情和预览都用它。
    pub source: String,
    pub id: String,
    pub title: String,
    pub summary: String,
    pub icon: Option<PluginIcon>,
    pub category: Option<PluginCategory>,
    pub tags: Vec<String>,
    pub kind: PluginKind,
    /// 来源最新的版本；官方插件是索引里的版本。
    pub version: String,
    pub trust: PluginTrust,
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
    pub category: Option<PluginCategory>,
    pub tags: Vec<String>,
    pub kind: PluginKind,
    /// 详情说的版本。
    pub version: String,
    pub trust: PluginTrust,
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
    /// 对宿主的要求，一项一行，如“需要 2 GB 内存”。
    pub requirements: Vec<String>,
    /// 装好后有网页（小程序）。
    pub has_web: bool,
    /// 装着时是它的状态；没装时为空。
    pub installed: Option<InstalledPlugin>,
    /// 装不了的原因；能装时为空。
    pub blocked: Option<String>,
    /// 这个版本有要主人打开开关才开的局域网端口时是开关的情况；`enabled` 是当前值，没装时是 `false`。
    pub lan_webui: Option<LanWebuiOffer>,
    /// 这个插件的 Bot 已获授权存文件的路径，如 `music/周杰伦`（ADR-0027）。
    #[serde(default)]
    pub save_grants: Vec<String>,
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
}

/// `POST /_wonton/api/plugins/preview` 的请求体（JSON）：按来源预览。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct InstallSource {
    /// `github:<owner>/<repo>`，可以带 `@<tag>` 或 `@<40 位提交号>`；不带时是最新的正式版本。
    pub source: String,
}

/// 安装或升级前给主人确认的内容：预览接口的响应。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct InstallPreview {
    /// 解析后的来源，带着 tag 或提交，如 `github:owner/repo@v1.2.0`；上传的插件包为空。
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
    pub icon: Option<PluginIcon>,
    pub trust: PluginTrust,
    pub author: Option<PluginAuthor>,
    /// 要授予的全部权限；主人确认的就是这些。
    pub permissions: Vec<PermissionLine>,
    /// 确认前要看到的其他说明：未审核、镜像没有按摘要固定、对宿主的要求等，按顺序显示，`emphasis` 的醒目显示。
    pub notices: Vec<CardLine>,
    /// 升级时是装着的版本；新装时为空。
    pub upgrade_from: Option<String>,
    /// 这个版本的更新说明。
    pub release_notes: Option<String>,
    /// 装不了的原因；能装时为空，确认按钮这时不可点。
    pub blocked: Option<String>,
    /// 有要主人打开开关才开的局域网端口时是开关的情况，确认页上显示；主人的选择放在 `lan_webui` 请求字段带回。
    pub lan_webui: Option<LanWebuiOffer>,
}

/// `POST /_wonton/api/plugins` 的请求体（JSON）：按来源安装。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct InstallRequest {
    /// 预览给的 `source`。
    pub source: String,
    /// 预览给的 `digest`；对不上时得到 409 `changed`。
    pub digest: String,
    /// 主人确认的权限：预览里列出的全部权限，少一项多一项都装不上。
    pub grant: Vec<String>,
    /// 打开开关 `lan_webui`。预览没有 [`LanWebuiOffer`] 时不起作用。
    #[serde(default)]
    pub lan_webui: bool,
}

/// `POST /_wonton/api/plugins/{id}/upgrade` 的请求体（JSON）：升到升级预览给的版本。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct UpgradeRequest {
    pub digest: String,
    pub grant: Vec<String>,
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
}

/// 和插件有关的操作失败了。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct PluginProblem {
    pub reason: PluginFailure,
    /// 核心写的说明，壳原样显示。
    pub message: String,
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
    /// 家里的服务器选不出局域网地址，申请了 `network.lan` 的插件装不上：要主人在局域网设置里选网卡或地址。
    LanUnavailable,
    /// 家里的服务器正在升级，升级结束前不能动插件（ADR-0025）。
    CoreUpgrading,
    /// 主人配的代理连不上或者拒绝了，出网插件装不上：要主人在代理设置里改地址，或者修好代理（ADR-0023）。
    EgressProxyUnavailable,
    /// 家里的服务器上出错了：Docker、身份提供方、Matrix、模型网关没就绪，或者插件没通过健康检查。
    Upstream,
    /// App 和家里服务器的版本对不上，有一边要更新。
    Protocol,
}

/// 一次安装、升级或卸载。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct PluginTask {
    /// 插件 ID。
    pub id: String,
    pub title: String,
    pub icon: Option<PluginIcon>,
    pub operation: PluginOperation,
    pub state: PluginTaskState,
}

/// 任务做的是什么。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum PluginOperation {
    Install,
    Upgrade,
    Uninstall,
}

/// 任务的进展。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PluginTaskState {
    /// 家里的服务器在做；App 退到后台或者断开了，它也照样做完。
    Running,
    /// 做完了，已安装列表随之更新。
    Succeeded,
    Failed {
        reason: PluginFailure,
        message: String,
    },
    /// 等不到结果：家里的服务器可能已经做完了，要看已安装列表。
    Uncertain { message: String },
}

#[cfg(test)]
mod tests;
