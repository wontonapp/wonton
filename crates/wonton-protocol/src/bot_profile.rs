//! Bot 的名字、头像和状态头像（ADR-0044）。
//!
//! - 名字和静态头像是 Bot 账号的 Matrix 资料 `displayname`、`avatar_url`；状态头像是资料里的自定义字段
//!   [`AVATARS_FIELD`]（MSC4133），格式是 [`StateAvatars`]。会说 Matrix 的 Agent 也能自己写它。
//! - 核心的管理接口：[`BOT_ACTIVITY_PATH`] 长轮询各 Bot 的事实（[`BotActivityReport`]），
//!   [`bot_profile_path`] 由主人改资料（[`BotProfileUpdate`]）。
//! - 客户端核心把事实和 Matrix 的 typing 合成 [`BotActivity`]，连同资料放进 [`BotView`]。
//!
//! 核心可能比 App 新：核心多给的字段被忽略；枚举遇到不认识的取值时落到 `Other`。

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{BotChatSettings, BotRef, BotWebView, MediaHandle};

/// Bot 资料里状态头像那个字段的名字。
pub const AVATARS_FIELD: &str = "app.wonton.avatars";

/// `GET <base>/_wonton/api/bots/activity?after=<rev>`：比 `after` 新的 [`BotActivityReport`]；没有更新的就挂住，
/// 最多 [`BOT_ACTIVITY_WAIT_SECS`] 秒后返回当前的。不带 `after` 时立即返回。
pub const BOT_ACTIVITY_PATH: &str = "/_wonton/api/bots/activity";

/// 长轮询最多挂住多久。客户端的超时要比它长。
pub const BOT_ACTIVITY_WAIT_SECS: u64 = 25;

/// 状态头像、静态头像的边长上限（像素）。必须是方形。
pub const MAX_AVATAR_EDGE: u32 = 640;
/// 每个头像文件的大小上限。
pub const MAX_AVATAR_BYTES: u64 = 2 * 1024 * 1024;
/// 动图和视频的时长上限。
pub const MAX_AVATAR_DURATION_MS: u32 = 6_000;
/// 名字最多几个字符（Unicode 标量）。
pub const MAX_BOT_NAME_CHARS: usize = 32;

/// `PUT <base>/_wonton/api/bots/<插件 ID>/profile`：主人改这个插件的 Bot 的资料。
pub fn bot_profile_path(plugin_id: &str) -> String {
    format!("/_wonton/api/bots/{plugin_id}/profile")
}

/// `GET` 取、`PUT` 改（请求体 [`BotPluginAccessUpdate`]），都返回 [`BotPluginAccess`]。只有主人能用。
pub fn bot_plugin_access_path(plugin_id: &str, bot: &str, target_plugin_id: &str) -> String {
    format!("/_wonton/api/bots/{plugin_id}/{bot}/plugin-access/{target_plugin_id}")
}

/// Bot 资料页里的一项工具插件或技能。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct BotPluginAccess {
    pub plugin_id: String,
    pub source: AccessSource,
    pub enabled: bool,
}

/// 插件因全局范围还是指定范围对这个 Bot 生效。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum AccessSource {
    Global,
    Specified,
    #[serde(other)]
    Other,
}

/// [`bot_plugin_access_path`] 的请求体，单独关掉会覆盖插件范围。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct BotPluginAccessUpdate {
    pub enabled: bool,
}

/// Bot 此刻在做什么，也是状态头像的状态（[`StateAvatars::states`] 的键）。客户端核心合成（规则见 ADR-0044），
/// 壳按它选状态头像。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum BotActivity {
    Idle,
    /// 有模型请求或工具调用在进行，或在聊天里“正在输入”。
    Working,
    /// 在等主人回答确认卡片。
    Waiting,
    /// 插件没在运行：在安装、升级、启动，停了或者出错。
    Offline,
}

impl BotActivity {
    pub const ALL: [Self; 4] = [Self::Idle, Self::Working, Self::Waiting, Self::Offline];

    /// 在 [`StateAvatars::states`] 里的键。
    pub fn key(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Working => "working",
            Self::Waiting => "waiting",
            Self::Offline => "offline",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|state| state.key() == key)
    }
}

/// 资料字段 [`AVATARS_FIELD`] 的值。键是 [`BotActivity::key`]；不认识的键原样保留，旧 App 忽略它们。主人或 Agent 清掉
/// 全部状态头像时写成空表而不是删掉字段：字段在，核心就不再按清单补默认素材。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StateAvatars {
    #[serde(default)]
    pub states: BTreeMap<String, AvatarMedia>,
}

impl StateAvatars {
    /// 这个状态显示哪一个：没写的用 `idle` 的；`idle` 也没有时为空，壳用静态头像。
    pub fn for_state(&self, state: BotActivity) -> Option<&AvatarMedia> {
        self.states
            .get(state.key())
            .or_else(|| self.states.get(BotActivity::Idle.key()))
    }
}

/// 一个头像文件：Matrix 媒体库里的一个 `mxc://`。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct AvatarMedia {
    /// `mxc://` 地址。
    pub url: String,
    /// 核心按文件头认出的类型，如 `video/mp4`、`image/webp`。
    pub mimetype: String,
    #[serde(rename = "w")]
    pub width: u32,
    #[serde(rename = "h")]
    pub height: u32,
    /// 字节数。
    pub size: u64,
    /// 动图和视频的时长；静态图为空。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u32>,
    /// 视频的静态首帧（`mxc://`）；没有时壳自己从视频取。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub poster: Option<String>,
}

/// 头像文件的种类，壳按它选播放方式。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum AvatarKind {
    /// PNG、JPEG、静态 WebP。
    Still,
    /// GIF、动图 WebP。
    Animated,
    /// MP4（H.264），静音循环。
    Video,
}

/// [`BOT_ACTIVITY_PATH`] 的响应：核心看得到的事实，合成状态在客户端核心。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct BotActivityReport {
    /// 任何一个 Bot 的事实变了就加一；下次请求带上它。核心重启后从头算，客户端见到变小了就当成新的。
    pub rev: u64,
    pub bots: Vec<BotActivityFacts>,
}

/// 一个 Bot 此刻的事实；不包含按请求者计算的网页访问权限。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct BotActivityFacts {
    pub plugin_id: String,
    /// 核心记录的 Bot 名；缺少时表示未提供，不从 Matrix 用户名反推，也不补成默认 Bot。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bot: Option<String>,
    /// Bot 的 Matrix 用户 ID。
    pub user_id: String,
    /// 插件在运行，健康检查过了。
    pub running: bool,
    /// 经核心的模型网关入口、还没结束的请求数（流式的到最后一块为止）。
    pub model_requests: u32,
    /// 经 MCP 总线、还没返回的工具调用数，不含在等主人确认的。
    pub tool_calls: u32,
    /// 这个 Bot 发起、在等主人回答的确认卡片数（MCP 工具和 ACP 的权限请求）。
    pub waiting: u32,
    /// 核心改过这个 Bot 的资料就加一，客户端见到变了就重新取资料。
    pub profile_rev: u64,
}

/// [`bot_profile_path`] 的请求体。没写的项不改。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BotProfileUpdate {
    /// 新名字，过 [`check_bot_name`]。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// 静态头像。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avatar: Option<AvatarChange>,
    /// 改哪几个状态头像。
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub states: BTreeMap<BotActivity, AvatarChange>,
}

/// 换成一个已经传到媒体库的文件，或者清掉。核心按文件头检查它（[`MAX_AVATAR_EDGE`] 等）。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum AvatarChange {
    Set {
        /// `mxc://` 地址。
        url: String,
        /// 视频的首帧（`mxc://`）。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        poster: Option<String>,
    },
    Clear,
}

/// [`bot_profile_path`] 的响应。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BotProfileUpdated {
    pub profile_rev: u64,
}

/// 名字不行的原因。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum BotNameProblem {
    Empty,
    /// 超过 [`MAX_BOT_NAME_CHARS`]。
    TooLong,
    /// 有换行或控制字符。
    ControlCharacters,
    /// 和核心账号或主人的名字一样（不分大小写）：Bot 不能冒充它们。只有家里的服务器查得出来。
    Reserved,
    #[serde(other)]
    Other,
}

/// 名字本身的规则：家里的服务器保存时查，App 在提交之前也用它查。`name` 已经去掉首尾空白。和核心账号、主人重名
/// （[`BotNameProblem::Reserved`]）由家里的服务器另查，它知道这两个名字。
pub fn check_bot_name(name: &str) -> Result<(), BotNameProblem> {
    if name.is_empty() {
        return Err(BotNameProblem::Empty);
    }
    if name.chars().count() > MAX_BOT_NAME_CHARS {
        return Err(BotNameProblem::TooLong);
    }
    if name.chars().any(char::is_control) {
        return Err(BotNameProblem::ControlCharacters);
    }
    Ok(())
}

/// App 里显示的一个 Bot：已装插件的 Bot，资料来自 Matrix，状态来自核心和 typing。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct BotView {
    pub user_id: String,
    pub bot: BotRef,
    /// 由当前请求者的身份决定；未核实、断线或切换账号时为空，不能借用插件的默认网页地址。
    #[serde(default)]
    pub web: Option<BotWebView>,
    /// Matrix 显示名；还没取到资料时是插件名。
    pub name: String,
    /// 静态头像；没设时为空，壳用插件图标。
    pub avatar: Option<AvatarImage>,
    /// 设了头像的状态，按 [`BotActivity::ALL`] 的顺序。
    pub state_avatars: Vec<StateAvatar>,
    /// 为空表示不确定：App 没连上家里的服务器，或最近没取到状态。这时只显示静态头像。
    pub activity: Option<BotActivity>,
    /// 主人在 App 里改的资料还在提交。
    pub saving: bool,
    /// 最近一次改资料失败；壳显示后用 [`Action::DismissBotProfileProblem`](crate::Action::DismissBotProfileProblem) 清掉。
    pub problem: Option<BotProfileProblem>,
    /// Bot 资料页“显示”一节（ADR-0060）。
    pub chat: BotChatSettings,
}

impl BotView {
    /// 现在该播的状态头像：这个状态的，没有时用待机的；为空时显示静态头像。
    pub fn current_state_avatar(&self) -> Option<&AvatarImage> {
        let find = |state: BotActivity| {
            self.state_avatars
                .iter()
                .find(|a| a.state == state)
                .map(|a| &a.image)
        };
        let state = self.activity?;
        find(state).or_else(|| find(BotActivity::Idle))
    }
}

/// 一个状态的头像。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct StateAvatar {
    pub state: BotActivity,
    pub image: AvatarImage,
}

/// 壳要显示的一个头像。字节用 `load_media` 取，视频用 `avatar_file` 取本机文件。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct AvatarImage {
    pub media: MediaHandle,
    pub kind: AvatarKind,
    /// 静态头像（`avatar_url`）不知道宽高，为空。
    pub width: Option<u32>,
    pub height: Option<u32>,
    /// 视频的首帧；没有时壳自己从视频取。
    pub poster: Option<MediaHandle>,
}

/// 主人在 App 里选好的头像文件，壳已经按 [`MAX_AVATAR_EDGE`] 等裁好、转好。
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct AvatarUpload {
    /// 如 `image/jpeg`、`video/mp4`。
    pub mime_type: String,
    pub data: Vec<u8>,
    /// 视频的首帧，JPEG；壳取不到时为空。
    pub poster: Option<Vec<u8>>,
}

impl std::fmt::Debug for AvatarUpload {
    /// 只打字节数。
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AvatarUpload")
            .field("mime_type", &self.mime_type)
            .field("bytes", &self.data.len())
            .field("poster_bytes", &self.poster.as_ref().map(Vec::len))
            .finish()
    }
}

/// 改资料失败。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct BotProfileProblem {
    pub reason: BotProfileFailure,
    /// 诊断信息（英文），放进默认收起的“详情”。
    pub detail: String,
}

/// 改资料失败的类别，壳按它写文字。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BotProfileFailure {
    /// 连不上家里的服务器。
    TransportUnavailable,
    /// 插件没装、没有 Bot，或 Matrix 还没登录好。
    NotReady,
    /// 名字不行。
    InvalidName { problem: BotNameProblem },
    /// 文件不行。
    MediaRejected { problem: AvatarMediaProblem },
    /// 家里的服务器或 Matrix 出错。
    Upstream,
    /// 只有主人的接口，家人的设备改不了（ADR-0045）。
    PermissionRequired,
    #[serde(other)]
    Other,
}

/// 头像文件不行的原因（核心的 `media_rejected` 错误也带它）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum AvatarMediaProblem {
    /// 不是支持的格式，或者文件头和内容对不上。
    Format,
    /// 超过 [`MAX_AVATAR_BYTES`]。
    TooLarge,
    /// 边长超过 [`MAX_AVATAR_EDGE`]。
    TooBig,
    NotSquare,
    /// 超过 [`MAX_AVATAR_DURATION_MS`]。
    TooLong,
    /// 读不出宽高或时长。
    Unreadable,
    #[serde(other)]
    Other,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn the_profile_field_has_a_stable_wire_format() {
        let value = json!({"states": {
            "idle": {"url": "mxc://h/a", "mimetype": "video/mp4", "w": 480, "h": 480, "size": 1000,
                     "duration_ms": 3000, "poster": "mxc://h/p"},
            "dancing": {"url": "mxc://h/b", "mimetype": "image/png", "w": 64, "h": 64, "size": 10},
        }});
        let avatars: StateAvatars = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(serde_json::to_value(&avatars).unwrap(), value);
        assert_eq!(
            avatars.for_state(BotActivity::Waiting).unwrap().url,
            "mxc://h/a"
        );
    }

    #[test]
    fn a_state_without_media_falls_back_to_idle_then_nothing() {
        let empty = StateAvatars::default();
        assert!(empty.for_state(BotActivity::Idle).is_none());
        assert!(empty.for_state(BotActivity::Working).is_none());
    }

    #[test]
    fn a_newer_core_does_not_break_the_report() {
        let report: BotActivityReport = serde_json::from_value(json!({
            "rev": 3, "future": true,
            "bots": [{"plugin_id": "a.b", "user_id": "@b:h", "running": true, "gpu": 1,
                      "model_requests": 1, "tool_calls": 0, "waiting": 0, "profile_rev": 2}],
        }))
        .unwrap();
        assert_eq!(report.bots[0].model_requests, 1);
    }

    #[test]
    fn reports_keep_named_bots_without_inventing_missing_names() {
        let report: BotActivityReport = serde_json::from_value(json!({
            "rev": 4,
            "bots": [
                {"plugin_id": "a.b", "user_id": "@b:h", "running": true,
                 "model_requests": 1, "tool_calls": 0, "waiting": 0, "profile_rev": 2},
                {"plugin_id": "a.b", "bot": "alice", "user_id": "@b-alice:h", "running": true,
                 "model_requests": 0, "tool_calls": 2, "waiting": 1, "profile_rev": 3, "future": true},
            ],
        }))
        .unwrap();
        assert_eq!(report.bots[0].plugin_id, "a.b");
        assert_eq!(report.bots[0].bot, None);
        assert_eq!(report.bots[1].plugin_id, "a.b");
        assert_eq!(report.bots[1].bot.as_deref(), Some("alice"));
        assert_eq!(report.bots[1].tool_calls, 2);
        let encoded = serde_json::to_value(&report).unwrap();
        assert_eq!(encoded["bots"][0]["plugin_id"], "a.b");
        assert!(encoded["bots"][0].get("bot").is_none());
        assert_eq!(encoded["bots"][1]["bot"], "alice");
    }

    #[test]
    fn an_update_names_only_what_changes() {
        let update = BotProfileUpdate {
            name: Some("小云".into()),
            avatar: None,
            states: BTreeMap::from([
                (
                    BotActivity::Working,
                    AvatarChange::Set {
                        url: "mxc://h/t".into(),
                        poster: None,
                    },
                ),
                (BotActivity::Offline, AvatarChange::Clear),
            ]),
        };
        let value = serde_json::to_value(&update).unwrap();
        assert_eq!(
            value,
            json!({"name": "小云", "states": {
                "working": {"action": "set", "url": "mxc://h/t"},
                "offline": {"action": "clear"},
            }})
        );
        assert_eq!(
            serde_json::from_value::<BotProfileUpdate>(value).unwrap(),
            update
        );
    }

    #[test]
    fn a_name_is_one_short_line() {
        assert_eq!(check_bot_name("小云"), Ok(()));
        assert_eq!(check_bot_name(""), Err(BotNameProblem::Empty));
        assert_eq!(
            check_bot_name("a\nb"),
            Err(BotNameProblem::ControlCharacters)
        );
        let long = "云".repeat(MAX_BOT_NAME_CHARS + 1);
        assert_eq!(check_bot_name(&long), Err(BotNameProblem::TooLong));
    }

    #[test]
    fn unknown_activity_shows_no_state_avatar() {
        let image = |url: &str| AvatarImage {
            media: MediaHandle { token: url.into() },
            kind: AvatarKind::Video,
            width: Some(480),
            height: Some(480),
            poster: None,
        };
        let mut bot = BotView {
            chat: Default::default(),
            user_id: "@b:h".into(),
            bot: BotRef {
                plugin_id: "a.b".into(),
                bot: "default".into(),
            },
            web: None,
            name: "B".into(),
            avatar: None,
            state_avatars: vec![StateAvatar {
                state: BotActivity::Idle,
                image: image("idle"),
            }],
            activity: None,
            saving: false,
            problem: None,
        };
        assert!(bot.current_state_avatar().is_none());
        bot.activity = Some(BotActivity::Working);
        assert_eq!(
            bot.current_state_avatar().unwrap().media.token,
            "idle",
            "没设的状态用待机的"
        );
    }
}
