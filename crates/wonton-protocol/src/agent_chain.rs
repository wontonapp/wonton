//! 调用链（ADR-0061 第 10 节）：一句话引出的所有 Agent 会话是一条链，链上记身份、层数和轮数。
//!
//! Bot 插件每轮开始时 `POST` [`AGENT_CHAINS_PATH`]，请求体是 [`ChainTurnReport`]，拿到 [`ChainTurnGrant`] 才
//! 发给 Agent；被拒时拿到 [`ChainError`]。说话人用 [`WONTON_ON_BEHALF_OF_HEADER`] 报到 MCP 请求上。
//! 链代表的人和链上各 Bot 的归属人随时能停整条链；停了之后的一切新请求都是
//! [`ChainError::Stopped`]。

use serde::{Deserialize, Serialize};

use crate::{BotRef, ConfirmationOrigin};

/// Bot 插件每轮开始和结束报告的接口路径（`POST`，和 [`AGENT_PERMISSION_PATH`](crate::AGENT_PERMISSION_PATH) 同一道门）。
pub const AGENT_CHAINS_PATH: &str = "/agent/chains";

/// Bot 插件取批准的消息和要取消的轮次的接口路径（`GET`，长轮询，最多 [`AGENT_OUTBOX_WAIT_SECS`] 秒，和 [`AGENT_PERMISSION_PATH`](crate::AGENT_PERMISSION_PATH)
/// 同一道门），回报见 [`crate::OutboxReceipt`]。
pub const AGENT_OUTBOX_PATH: &str = "/agent/outbox";

/// 每轮消息的说话人（MXID）：Bot 插件调用工具时带上。核心只在这个 Bot 听的人里采用，否则按只读、
/// 联系人门按远端；没带时从已报的轮次取，轮次也没有就按只读，不按 Bot 归属人。
pub const WONTON_ON_BEHALF_OF_HEADER: &str = "X-Wonton-On-Behalf-Of";

/// 核心注入的请求头：链上目标 Bot 是哪一个，网关按它起对应的 Agent 进程。
pub const WONTON_BOT_HEADER: &str = "X-Wonton-Bot";

/// Bot 插件给 Wonton MCP 的请求头里放的会话 ID（ADR-0061 第 10 节），核心按它找到当前这一轮的链。
/// 和 [`ORIGIN_ROOM_HEADER`](crate::ORIGIN_ROOM_HEADER)、[`ORIGIN_THREAD_HEADER`](crate::ORIGIN_THREAD_HEADER) 并列。
pub const SESSION_HEADER: &str = "wonton-session";

/// Bot 插件证明“这一轮是哪个 Bot 在报”的请求头（ADR-0083 第 5 节）：值是这个 Bot 的 Matrix 访问令牌。总线令牌 Agent
/// 自己也拿得到，只认来源地址和它不够。
pub const BOT_TOKEN_HEADER: &str = "X-Wonton-Bot-Token";

/// [`AGENT_OUTBOX_PATH`] 长轮询最多挂多少秒。
pub const AGENT_OUTBOX_WAIT_SECS: u64 = 25;

/// [`AGENT_CHAINS_API_PATH`] 带 `after` 长轮询时最多挂多少秒。
pub const AGENT_CHAINS_WAIT_SECS: u64 = 25;

/// App 的链列表（长轮询，带 `after=<revision>`），和确认列表一样的做法。
pub const AGENT_CHAINS_API_PATH: &str = "/_wonton/api/agent/chains";

/// 停一条链：`POST`，路径参数是链 ID。
pub fn agent_chain_stop_path(chain: &str) -> String {
    format!("/_wonton/api/agent/chains/{chain}/stop")
}

/// 改设置“Agent”一页的上限（[`ChainLimits`]，`PUT`），只有主人能改。
pub const AGENT_LIMITS_PATH: &str = "/_wonton/api/agent/limits";

/// 设置“Agent”一页的当前值（[`AgentSettingsViewModel`]）。
pub const AGENT_SETTINGS_PATH: &str = "/_wonton/api/agent/settings";

/// [`AGENT_CHAINS_PATH`] 的请求体。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "phase", rename_all = "snake_case")]
pub enum ChainTurnReport {
    /// 一轮开始：拿到 [`ChainTurnGrant`] 才发 `session/prompt`。
    Start {
        /// Bot 插件自己的会话 ID；同一会话的下一句话再报 Start 是另一条链。
        session: String,
        /// 说话人在哪个房间。
        room: String,
        /// 话题（Matrix thread）的根事件 ID；不在话题里时不带。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        thread: Option<String>,
        /// 说话人。核心只认这个 Bot 听的人（ADR-0045），不是就 [`ChainError::PermissionDenied`]。
        sender: String,
        /// 触发这一轮的那条消息的事件 ID：点名叫回沿用原链时按它找链。
        event_id: String,
    },
    /// 这一轮结束（成功、取消、出错都算）：链的轮数已经记下。
    End { session: String },
}

/// [`ChainTurnReport::Start`] 的响应：链 ID 和这一轮在第几层。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ChainTurnGrant {
    /// 128 位随机的链 ID，核心发。
    pub chain: String,
    /// 第几层：人直接对话的编排器或第一个 Agent 是第 1 层（ADR-0061 第 10 节）。
    pub layer: u32,
}

/// 为什么不给链。错误响应体就是这个类型，类别和 ADR-0061 第 1 节的能力契约一致。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ChainError {
    /// 到上限了：层数或轮数。给一份新的要链代表的人和链上各 Bot 的归属人都同意（上限卡片）。
    LimitReached { kind: ChainLimitKind },
    /// 链被人停了；要接着做，人重新说一句开一条新链。
    Stopped,
    /// 说话人不是这个 Bot 听的人，或目标 Bot 不听链代表的人。
    PermissionDenied,
    /// 这一轮不能进链（[`ChainRefusal`]）：被点名的 Bot 报 start 时点名者不在链上。
    /// 不归 [`ChainError::LimitReached`]，不能用“再给一份”放开（ADR-0061 第 10 节）。
    InvalidInput { refusal: ChainRefusal },
    /// 比调用方新的核心给的未知类别。
    #[serde(other)]
    Other,
}

/// 到的是哪一种上限。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum ChainLimitKind {
    /// 第 4 层起核心拒绝（默认最多 3 层）；层从点名者继承后链上全是第 1 层，不再触发。
    Depth,
    /// 链上所有轮次加起来到了上限。
    Rounds,
    /// 比调用方新的核心给的未知上限。
    #[serde(other)]
    Other,
}

/// [`ChainError::InvalidInput`] 的原因。账本只按唤醒标签找链、不记祖先路径，核心只返回 `Other`；
/// `SelfCall`、`Loop` 是调用方自己判断时用的取值。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum ChainRefusal {
    /// 目标 Bot 就是发起的 Bot（ADR-0061 第 9 节）；同一个插件的另一个 Bot 可以。
    SelfCall,
    /// 目标在调用方会话的祖先路径上（A→B→A）；同链兄弟和重复交给 B 不算绕圈。
    Loop,
    /// 比调用方新的核心给的未知原因。
    #[serde(other)]
    Other,
}

/// 链的上限（ADR-0061 第 10 节）：设置里只有主人能改（[`AgentAction::SetAgentLimits`]）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ChainLimits {
    /// 最多几层：A→B→C 是 3 层。层从点名者继承，账本不再按它拒绝。
    pub max_depth: u32,
    /// 链上所有轮次加起来最多多少轮。
    pub max_rounds: u32,
}

impl Default for ChainLimits {
    fn default() -> Self {
        Self {
            max_depth: 3,
            max_rounds: 50,
        }
    }
}

/// App 里 Agent 链列表的状态（[`AgentChainsViewModel`]）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum AgentChainStatus {
    /// 有轮次在跑，或刚被“再给一份”放开、等下一轮。
    Running,
    Stopped,
    /// 到了上限，等上限卡片的回答。
    LimitReached,
    /// 做完了：最后一轮报了结束，或核心重启后连接都断了（ADR-0061 第 2 节）。在 App 的
    /// 进行中列表里不出现；点名叫回会把它叫回 `Running`，停它则变成 `Stopped`。
    Ended,
    /// 比壳新的核心给的未知状态。
    #[serde(other)]
    Other,
}

/// 链上的一个 Bot 和它的层。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct AgentChainBot {
    pub bot: BotRef,
    /// 第几层：人直接对话的是第 1 层，它派的是第 2 层（ADR-0061 第 10 节）。
    pub layer: u32,
}

/// 链列表的一行。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct AgentChainRow {
    /// 链 ID。
    pub chain: String,
    /// 链代表的人：最先说话的那个人，之后每一跳都替他说话（ADR-0061 第 10 节）。
    pub user: String,
    /// 链从哪个对话开始。
    pub origin: ConfirmationOrigin,
    /// 链上的 Bot，按层排。
    pub bots: Vec<AgentChainBot>,
    /// 链上所有轮次的总数。
    pub rounds: u32,
    pub status: AgentChainStatus,
    /// 链代表的人和链上各 Bot 的归属人能停（[`AgentAction::StopChain`]）；别人看不到这一行。
    pub can_stop: bool,
    /// 链上有轮次在跑（Bot 报了开始、没报结束，链也没停）：这时停链才打断得到正在做的事。
    /// 比客户端旧的核心不带这个键，按没有在跑算。
    #[serde(default)]
    #[cfg_attr(feature = "uniffi", uniffi(default = false))]
    pub live: bool,
}

/// App 的 Agent 链列表（ADR-0061 第 10 节）。
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct AgentChainsViewModel {
    /// 还没结束的在前，同状态的按最近活动排；结束的留给链的详情，不进列表。
    pub chains: Vec<AgentChainRow>,
    /// 列表的版本，任何一条链变了就变大；下次带 `after=<revision>` 长轮询。核心重启后从头数。
    #[serde(default)]
    pub revision: u64,
    /// 最近一次动作失败及其原因；再次发送动作时清除。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action_failure: Option<AgentActionFailure>,
}

/// App 发的 Agent 链动作（ADR-0061 第 10 节）。它通过统一的 `Action::Agent` 包装进入客户端核心。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentAction {
    /// 停下整条链：链代表的人和链上各 Bot 的归属人可以，别的设备返回 `Unauthorized`。停下的链不再接
    /// 新的轮次和请求；链列表里那一行的状态变成 [`AgentChainStatus::Stopped`]。轮次刚好报完结束、
    /// 链已经记成做完时也停得成（同样变成 `Stopped`，回给 App 的也是这一行）。
    StopChain { chain: String },
    /// 停下这条时间线里 Bot 正在跑的轮次：客户端核心把从这里开始、还有轮次在跑的链都停掉，进展和结果
    /// 在 [`RoomAgentStop`](crate::RoomAgentStop) 里，不进 [`AgentChainsViewModel::action_failure`]。
    StopTurns { target: crate::TimelineTarget },
    /// 改设置“Agent”一页的上限（层数、轮数），只有主人能改，别的设备返回 `Unauthorized`。
    SetAgentLimits { limits: ChainLimits },
}

/// Agent 动作的失败及其分类。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct AgentActionFailure {
    pub action: AgentAction,
    pub error: crate::CoreError,
}

/// 设置“Agent”一页（ADR-0061 第 10 节）：层数、轮数上限，只有主人能改。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct AgentSettingsViewModel {
    pub limits: ChainLimits,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn a_start_report_has_a_stable_wire_format() {
        let start = ChainTurnReport::Start {
            session: "s-1".into(),
            room: "!r:home.example".into(),
            thread: Some("$root".into()),
            sender: "@alex:home.example".into(),
            event_id: "$e1".into(),
        };
        assert_eq!(
            json!(start),
            json!({
                "phase": "start",
                "session": "s-1",
                "room": "!r:home.example",
                "thread": "$root",
                "sender": "@alex:home.example",
                "event_id": "$e1",
            })
        );
        assert_eq!(
            serde_json::from_value::<ChainTurnReport>(json!(start)).unwrap(),
            start
        );

        let bare = ChainTurnReport::Start {
            session: "s-1".into(),
            room: "!r:home.example".into(),
            thread: None,
            sender: "@alex:home.example".into(),
            event_id: "$e1".into(),
        };
        assert_eq!(
            json!(bare),
            json!({
                "phase": "start",
                "session": "s-1",
                "room": "!r:home.example",
                "sender": "@alex:home.example",
                "event_id": "$e1",
            })
        );
    }

    #[test]
    fn an_end_report_names_only_the_session() {
        let end = ChainTurnReport::End {
            session: "s-1".into(),
        };
        assert_eq!(json!(end), json!({"phase": "end", "session": "s-1"}));
        assert_eq!(
            serde_json::from_value::<ChainTurnReport>(json!(end)).unwrap(),
            end
        );
    }

    #[test]
    fn a_grant_has_a_stable_wire_format() {
        let grant = ChainTurnGrant {
            chain: "c-1".into(),
            layer: 2,
        };
        assert_eq!(json!(grant), json!({"chain": "c-1", "layer": 2}));
        assert_eq!(
            serde_json::from_value::<ChainTurnGrant>(json!(grant)).unwrap(),
            grant
        );
    }

    /// 契约：错误带各自的原因，不认识的类别落到 `Other`，核心比 Bot 插件新时插件照样解析。
    #[test]
    fn errors_carry_their_kind_and_fall_back_to_other() {
        let depth = ChainError::LimitReached {
            kind: ChainLimitKind::Depth,
        };
        assert_eq!(
            json!(depth),
            json!({"type": "limit_reached", "kind": "depth"})
        );
        assert_eq!(
            serde_json::from_value::<ChainError>(json!(depth)).unwrap(),
            depth
        );
        assert_eq!(
            serde_json::from_value::<ChainError>(json!({"type": "limit_reached", "kind": "jobs"}))
                .unwrap(),
            ChainError::LimitReached {
                kind: ChainLimitKind::Other,
            }
        );
        let refusal = ChainError::InvalidInput {
            refusal: ChainRefusal::Loop,
        };
        assert_eq!(
            json!(refusal),
            json!({"type": "invalid_input", "refusal": "loop"})
        );
        assert_eq!(
            serde_json::from_value::<ChainError>(json!({"type": "name_collision"})).unwrap(),
            ChainError::Other
        );
    }

    #[test]
    fn limits_default_to_the_reviewed_values() {
        let limits = ChainLimits::default();
        assert_eq!(limits.max_depth, 3);
        assert_eq!(limits.max_rounds, 50);
        assert_eq!(
            serde_json::from_value::<ChainLimits>(json!({
                "max_depth": 3, "max_rounds": 50
            }))
            .unwrap(),
            limits
        );
        let settings = AgentSettingsViewModel { limits };
        assert_eq!(
            serde_json::from_value::<AgentSettingsViewModel>(json!({
                "limits": {"max_depth": 3, "max_rounds": 50}
            }))
            .unwrap(),
            settings
        );
    }

    /// 契约：动作的线上形状固定；通过统一 `Action::Agent` 入口传递。
    #[test]
    fn chain_actions_have_a_stable_wire_format() {
        let stop = AgentAction::StopChain {
            chain: "c-1".into(),
        };
        assert_eq!(json!(stop), json!({"type": "stop_chain", "chain": "c-1"}));
        assert_eq!(
            serde_json::from_value::<AgentAction>(json!(stop)).unwrap(),
            stop
        );
        let turns = AgentAction::StopTurns {
            target: crate::TimelineTarget {
                room_id: "!r:home.example".into(),
                thread_root: None,
            },
        };
        assert_eq!(
            json!(turns),
            json!({"type": "stop_turns", "target": {"room_id": "!r:home.example"}})
        );
        assert_eq!(
            serde_json::from_value::<AgentAction>(json!(turns)).unwrap(),
            turns
        );
        let limits = AgentAction::SetAgentLimits {
            limits: ChainLimits {
                max_depth: 5,
                max_rounds: 100,
            },
        };
        assert_eq!(
            json!(limits),
            json!({
                "type": "set_agent_limits",
                "limits": {"max_depth": 5, "max_rounds": 100},
            })
        );
        assert_eq!(
            serde_json::from_value::<AgentAction>(json!(limits)).unwrap(),
            limits
        );
    }

    /// 契约：行原样往返；不认识的状态落到 `Other`。
    #[test]
    fn rows_round_trip_and_unknown_states_fall_back() {
        let row = AgentChainRow {
            chain: "c-1".into(),
            user: "@alex:home.example".into(),
            origin: ConfirmationOrigin {
                room_id: "!r:home.example".into(),
                thread_id: None,
            },
            bots: vec![
                AgentChainBot {
                    bot: BotRef {
                        plugin_id: "app.orchestrator".into(),
                        bot: "owner".into(),
                    },
                    layer: 1,
                },
                AgentChainBot {
                    bot: BotRef {
                        plugin_id: "app.codex".into(),
                        bot: "owner".into(),
                    },
                    layer: 2,
                },
            ],
            rounds: 7,
            status: AgentChainStatus::Running,
            can_stop: true,
            live: true,
        };
        assert_eq!(
            serde_json::from_value::<AgentChainRow>(json!(row)).unwrap(),
            row
        );
        assert_eq!(
            serde_json::from_value::<AgentChainStatus>(json!("limit_reached")).unwrap(),
            AgentChainStatus::LimitReached
        );
        assert_eq!(
            serde_json::from_value::<AgentChainStatus>(json!("waking")).unwrap(),
            AgentChainStatus::Other
        );
    }
}
