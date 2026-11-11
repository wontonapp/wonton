//! 确认卡片是谁发起的、显示在哪个会话（ADR-0016 的“卡片显示在发起的会话里”）。
//!
//! 卡片不发进 Matrix 房间：核心记着卡片，经推送和 [`CONFIRMATIONS_PATH`] 交给 App，App 把它画在发起的会话里，
//! 以及“等你确认”列表里。
//!
//! 来源会话由 Agent 一侧报上来，是不可信输入：核心只认主人和发起的 Bot 都在的房间，没报或者对不上时落到 Bot 的
//! 主房间。来源只决定卡片显示在哪，不影响谁能回答（回答的门见 [`crate::answer_path`]）。

use serde::{Deserialize, Serialize};

use crate::{BotRef, ChatProblem, ConfirmationView};

/// MCP `tools/call` 请求的 `params._meta` 里报来源会话的键，值是 [`ConfirmationOrigin`]。
pub const ORIGIN_META_KEY: &str = "app.wonton/origin";

/// 报来源会话的 HTTP 请求头：MCP 客户端改不了每次调用的 `_meta`，但能给一个 MCP 服务器配固定请求头时用
/// （ACP 的 `session/new` 里 `mcpServers[].headers`，一个会话一份）。同一个请求两样都有时 `_meta` 为准。
pub const ORIGIN_ROOM_HEADER: &str = "wonton-origin-room";

/// 和 [`ORIGIN_ROOM_HEADER`] 一起用，报话题的根事件。
pub const ORIGIN_THREAD_HEADER: &str = "wonton-origin-thread";

/// `GET <核心地址>/_wonton/api/confirmations?after=<revision>`：[`ConfirmationList`](crate::ConfirmationList)。
/// 带 `after` 时没有比它新的就挂住，最多 [`CONFIRMATIONS_WAIT_SECS`] 秒后返回当前的；不带时立即返回。
pub const CONFIRMATIONS_PATH: &str = "/_wonton/api/confirmations";

/// [`CONFIRMATIONS_PATH`] 的长轮询最多挂住多久。客户端的超时要比它长。
pub const CONFIRMATIONS_WAIT_SECS: u64 = 25;

/// `GET` 一张卡片（[`ConfirmationCard`](crate::ConfirmationCard)）：等回答的、和结束不到 24 小时的都取得到，再早的 404。点开推送时用它。
pub fn confirmation_path(id: &str) -> String {
    format!("{CONFIRMATIONS_PATH}/{id}")
}

/// 一个会话：房间，加上可选的话题。
///
/// Agent 报上来时（[`ORIGIN_META_KEY`]、[`AgentPermissionRequest::origin`](crate::AgentPermissionRequest::origin)）
/// 没核对过；出现在卡片上时核心已经核对过。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ConfirmationOrigin {
    pub room_id: String,
    /// 话题（Matrix thread）的根事件 ID。App 没有话题界面时显示在 `room_id` 的主对话里。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "uniffi", uniffi(default = None))]
    pub thread_id: Option<String>,
}

/// 谁请主人确认。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ConfirmationRequester {
    /// 聊天里的 Bot。`user_id` 是它的 Matrix 账号，壳按它在 [`BotView`](crate::BotView) 里找名字和头像。
    Bot { bot: BotRef, user_id: String },
    /// 不进聊天的插件（没有 Bot）。
    Plugin { plugin_id: String },
    /// 比 App 新的核心写的发起者。
    #[serde(other)]
    Other,
}

/// 推送代理直接发给设备的确认推送（加密前的明文里的 `confirmation` 段，和服务器的 endpoint ID 放在一起）。
/// 卡片不是 Matrix 消息，没有房间里的事件；设备凭 `id` 经 [`confirmation_path`] 取卡片，取不到时也能凭这里的字
/// 弹通知、点开去对的会话。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceConfirmationPush {
    pub id: String,
    /// 卡片标题，如“Hermes 想安装 Grafana”。
    pub title: String,
    pub expires_at_ms: u64,
    /// 没有时（核心自己发起的，如服务器配对）点开去“等你确认”列表。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<ConfirmationOrigin>,
}

/// “等你确认”列表：代替原来的 Wonton 私聊，所有卡片都在这里，也能直接回答。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ConfirmationsViewModel {
    /// 等回答的，最早到期的在前。
    pub pending: Vec<ConfirmationView>,
    /// 结束不到 24 小时的（答了、拒了、过期、执行完），最近结束的在前。
    pub finished: Vec<ConfirmationView>,
    /// 还没取到过列表。
    pub loading: bool,
    /// 最近一次取列表没成：连不上、没权限。上面两个列表是上次取到的，可能已经过时，壳照实提示并给重试
    /// （[`Action::RetryConfirmations`](crate::Action::RetryConfirmations)）。
    pub problem: Option<ChatProblem>,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    /// 契约：Agent 在 `_meta` 里报的来源，话题可以不带。
    #[test]
    fn an_origin_has_a_stable_wire_format() {
        let origin: ConfirmationOrigin =
            serde_json::from_value(json!({"room_id": "!r:home.example"})).unwrap();
        assert_eq!(origin.thread_id, None);
        assert_eq!(json!(origin), json!({"room_id": "!r:home.example"}));
        let threaded = ConfirmationOrigin {
            room_id: "!r:home.example".into(),
            thread_id: Some("$root".into()),
        };
        assert_eq!(
            json!(threaded),
            json!({"room_id": "!r:home.example", "thread_id": "$root"})
        );
        assert_eq!(confirmation_path("abc"), "/_wonton/api/confirmations/abc");
    }

    /// 契约：核心比 App 新时，不认识的发起者落到 `Other`，卡片照样解析。
    #[test]
    fn an_unknown_requester_falls_back_to_other() {
        let bot: ConfirmationRequester = serde_json::from_value(json!({
            "type": "bot",
            "bot": {"plugin_id": "app.hermes", "bot": "owner"},
            "user_id": "@hermes:home.example",
        }))
        .unwrap();
        assert!(matches!(bot, ConfirmationRequester::Bot { .. }));
        let newer: ConfirmationRequester =
            serde_json::from_value(json!({"type": "peer_server", "name": "Alex"})).unwrap();
        assert_eq!(newer, ConfirmationRequester::Other);
    }

    #[test]
    fn a_device_push_without_origin_omits_it() {
        let push = DeviceConfirmationPush {
            id: "abc".into(),
            title: "Alex 的服务器想和你的服务器连接".into(),
            expires_at_ms: 1,
            origin: None,
        };
        assert_eq!(
            json!(push),
            json!({"id": "abc", "title": "Alex 的服务器想和你的服务器连接", "expires_at_ms": 1})
        );
    }
}
