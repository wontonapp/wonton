//! 调用链（ADR-0061 第 10 节）：一句话引出的所有 Agent 会话是一条链，链上记身份、层数、轮数和花费。
//!
//! 桥每轮开始时 `POST` [`AGENT_CHAINS_PATH`]，请求体是 [`ChainTurnReport`]，拿到 [`ChainTurnGrant`] 才发给
//! Agent；被拒时拿到 [`ChainError`]。核心转发 `agent_acp` 连接和模型请求时注入 [`WONTON_CHAIN_HEADER`]
//! 和上限头，网关改不了。链代表的人和链上各 Bot 的归属人随时能停整条链；停了之后的一切新请求都是
//! [`ChainError::Stopped`]。

use serde::{Deserialize, Serialize};

use crate::{BotRef, ConfirmationOrigin};

/// 桥每轮开始和结束报告的接口路径（`POST`，和 [`AGENT_PERMISSION_PATH`](crate::AGENT_PERMISSION_PATH) 同一道门）。
pub const AGENT_CHAINS_PATH: &str = "/agent/chains";

/// 前端取批准的消息和要取消的轮次的接口路径（`GET`，长轮询，和 [`AGENT_PERMISSION_PATH`](crate::AGENT_PERMISSION_PATH)
/// 同一道门），回报见 [`crate::OutboxReceipt`]。
pub const AGENT_OUTBOX_PATH: &str = "/agent/outbox";

/// 核心转发 `agent_acp` 连接时注入的请求头（ADR-0061 第 3、10 节）：哪个前端在连。使用方自己带的同名头被丢掉。
pub const WONTON_PLUGIN_HEADER: &str = "X-Wonton-Plugin";

/// 核心注入的请求头：这条连接替谁说话。运行时只认这个 Bot 听的人。
pub const WONTON_USER_HEADER: &str = "X-Wonton-User";

/// 编排器经自己的 Bot 连进来时带的请求头：链代表的人是谁。核心只在他是编排器的 Bot 听的人时采用。
pub const WONTON_ON_BEHALF_OF_HEADER: &str = "X-Wonton-On-Behalf-Of";

/// 核心注入的请求头：链上目标 Bot 是哪一个，网关按它起对应的 Agent 进程。
pub const WONTON_BOT_HEADER: &str = "X-Wonton-Bot";

/// 核心注入的请求头：这条连接属于哪条调用链。
pub const WONTON_CHAIN_HEADER: &str = "X-Wonton-Chain";

/// 上限头：核心转发时把链的层数上限给网关（ADR-0061 第 10 节），使用方改不了。
pub const WONTON_MAX_DEPTH_HEADER: &str = "X-Wonton-Max-Depth";

/// 上限头：链的轮数上限。
pub const WONTON_MAX_ROUNDS_HEADER: &str = "X-Wonton-Max-Rounds";

/// 上限头：链的花费上限（token）。
pub const WONTON_MAX_TOKENS_HEADER: &str = "X-Wonton-Max-Tokens";

/// 桥在 `session/new` 给 Wonton MCP 的请求头里放的会话 ID（ADR-0061 第 10 节），核心按它找到当前这一轮的链。
/// 和 [`ORIGIN_ROOM_HEADER`](crate::ORIGIN_ROOM_HEADER)、[`ORIGIN_THREAD_HEADER`](crate::ORIGIN_THREAD_HEADER) 并列。
pub const SESSION_HEADER: &str = "wonton-session";

/// 每个使用方同时开着的 `agent_acp` 连接数默认上限（ADR-0061 第 10 节）；满了返回 [`ChainError::Busy`]。
pub const DEFAULT_CONSUMER_AGENT_CONCURRENCY: u32 = 4;

/// [`AGENT_CHAINS_PATH`] 的请求体。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "phase", rename_all = "snake_case")]
pub enum ChainTurnReport {
    /// 一轮开始：拿到 [`ChainTurnGrant`] 才发 `session/prompt`。
    Start {
        /// ACP 会话 ID；同一会话的下一句话再报 Start 是另一条链。
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
    /// 到上限了：层数、轮数或花费。给一份新的要链代表的人和链上各 Bot 的归属人都同意（上限卡片）。
    LimitReached { kind: ChainLimitKind },
    /// 链被人停了；要接着做，人重新说一句开一条新链。
    Stopped,
    /// 这个使用方同时开着的连接数满了（ADR-0061 第 10 节）。
    Busy,
    /// 说话人不是这个 Bot 听的人，或目标 Bot 不听链代表的人。
    PermissionDenied,
    /// 自调（目标 Bot 就是发起的 Bot）或绕圈（链上已有的 Bot 又被调）。绕圈不归 [`ChainError::LimitReached`]，
    /// 不能用“再给一份”放开（ADR-0061 第 10 节）。
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
    /// 第 4 层起核心拒绝（默认最多 3 层）。
    Depth,
    /// 链上所有会话的 `session/prompt` 加起来到了上限。
    Rounds,
    /// 链上经模型网关的用量到了上限。
    Cost,
    /// 比调用方新的核心给的未知上限。
    #[serde(other)]
    Other,
}

/// [`ChainError::InvalidInput`] 的原因。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum ChainRefusal {
    /// 目标 Bot 就是发起的 Bot（ADR-0061 第 9 节）；同一个插件的另一个 Bot 可以。
    SelfCall,
    /// 链上已有的 Bot（含发起的那个）又被调（A→B→A），一律拒绝。
    Loop,
    /// 比调用方新的核心给的未知原因。
    #[serde(other)]
    Other,
}

/// 链的上限（ADR-0061 第 10 节）：核心转发时放在 [`WONTON_MAX_DEPTH_HEADER`] 等头里给网关，
/// 设置里只有主人能改（[`AgentChainAction::SetAgentLimits`]）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ChainLimits {
    /// 最多几层：A→B→C 是 3 层，再往下派被拒。
    pub max_depth: u32,
    /// 链上所有会话的 `session/prompt` 加起来最多多少轮。
    pub max_rounds: u32,
    /// 链上经模型网关的用量最多多少 token。
    pub max_tokens: u64,
}

impl Default for ChainLimits {
    fn default() -> Self {
        Self {
            max_depth: 3,
            max_rounds: 50,
            max_tokens: 2_000_000,
        }
    }
}

/// App 里 Agent 链列表的状态（[`AgentChainsViewModel`]）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum AgentChainStatus {
    Running,
    Stopped,
    /// 到了上限，等上限卡片的回答。
    LimitReached,
    /// 做完了，或核心重启后连接都断了（ADR-0061 第 2 节）。
    Ended,
    /// 比壳新的核心给的未知状态。
    #[serde(other)]
    Other,
}

/// 链的花费（ADR-0061 第 10 节）：只有经模型网关的用量记得到链上。
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ChainSpend {
    /// 经模型网关记账：累计 token；网关有价格时带金额，币种由网关定，壳原样显示。
    ViaGateway {
        tokens: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        amount: Option<f64>,
    },
    /// 记不上：网关没报用量、认不出是哪个 Bot，不猜。
    Unknown,
    /// 用官方登录的 Agent 不经网关（ADR-0061 第 10 节），花费只受轮数上限管。
    NotViaGateway,
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
    /// 链上所有会话的 `session/prompt` 总数。
    pub rounds: u32,
    pub spend: ChainSpend,
    pub status: AgentChainStatus,
    /// 链代表的人和链上各 Bot 的归属人能停（[`AgentChainAction::StopChain`]）；别人看不到这一行。
    pub can_stop: bool,
}

/// App 的 Agent 链列表（ADR-0061 第 10 节）。
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct AgentChainsViewModel {
    /// 还没结束的在前，同状态的按最近活动排；结束的留给链的详情，不进列表。
    pub chains: Vec<AgentChainRow>,
}

/// App 发的链动作：停链、改上限、改并发（ADR-0061 第 10 节）。客户端核心给一个单独的分派入口收它，
/// 和 [`Action`](crate::Action) 并列：[`Action`](crate::Action) 的 UniFFI 元数据（含文档，每变体几十字节）
/// 只剩下一个短名字的余量，链的动作放不进去，新作用域的动作开自己的枚举和入口。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentChainAction {
    /// 停下整条链：链代表的人和链上各 Bot 的归属人可以，别的设备返回 `Unauthorized`。停下的链不再接
    /// 新的轮次和请求；链列表里那一行的状态变成 [`AgentChainStatus::Stopped`]。
    StopChain { chain: String },
    /// 改设置“Agent”一页的上限（层数、轮数、花费），只有主人能改，别的设备返回 `Unauthorized`。
    SetAgentLimits { limits: ChainLimits },
    /// 改一个使用方（插件）同时能开多少条 `agent_acp` 连接（ADR-0061 第 10 节），只有主人能改。满了再连的
    /// 拿到 [`ChainError::Busy`]。
    SetConsumerAgentConcurrency { plugin_id: String, concurrency: u32 },
}

/// 设置“Agent”一页（ADR-0061 第 10 节）：层数、轮数、花费上限，只有主人能改。
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

    /// 契约：错误带各自的原因，不认识的类别落到 `Other`，核心比桥新时桥照样解析。
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
        assert_eq!(
            serde_json::from_value::<ChainError>(json!({"type": "busy"})).unwrap(),
            ChainError::Busy
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
        assert_eq!(limits.max_tokens, 2_000_000);
        assert_eq!(
            serde_json::from_value::<ChainLimits>(json!({
                "max_depth": 3, "max_rounds": 50, "max_tokens": 2000000
            }))
            .unwrap(),
            limits
        );
        let settings = AgentSettingsViewModel { limits };
        assert_eq!(
            serde_json::from_value::<AgentSettingsViewModel>(json!({
                "limits": {"max_depth": 3, "max_rounds": 50, "max_tokens": 2000000}
            }))
            .unwrap(),
            settings
        );
    }

    /// 契约：动作的线上形状固定；客户端核心给单独的入口收它（[`crate::Action`] 的元数据放不下）。
    #[test]
    fn chain_actions_have_a_stable_wire_format() {
        let stop = AgentChainAction::StopChain {
            chain: "c-1".into(),
        };
        assert_eq!(json!(stop), json!({"type": "stop_chain", "chain": "c-1"}));
        assert_eq!(
            serde_json::from_value::<AgentChainAction>(json!(stop)).unwrap(),
            stop
        );
        let limits = AgentChainAction::SetAgentLimits {
            limits: ChainLimits {
                max_depth: 5,
                max_rounds: 100,
                max_tokens: 4_000_000,
            },
        };
        assert_eq!(
            json!(limits),
            json!({
                "type": "set_agent_limits",
                "limits": {"max_depth": 5, "max_rounds": 100, "max_tokens": 4000000},
            })
        );
        assert_eq!(
            serde_json::from_value::<AgentChainAction>(json!(limits)).unwrap(),
            limits
        );
        let concurrency = AgentChainAction::SetConsumerAgentConcurrency {
            plugin_id: "app.orchestrator".into(),
            concurrency: 8,
        };
        assert_eq!(
            json!(concurrency),
            json!({
                "type": "set_consumer_agent_concurrency",
                "plugin_id": "app.orchestrator",
                "concurrency": 8,
            })
        );
        assert_eq!(
            serde_json::from_value::<AgentChainAction>(json!(concurrency)).unwrap(),
            concurrency
        );
    }

    /// 契约：花费三种都照实显示；不认识的状态落到 `Other`。
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
            spend: ChainSpend::ViaGateway {
                tokens: 12_345,
                amount: Some(0.02),
            },
            status: AgentChainStatus::Running,
            can_stop: true,
        };
        assert_eq!(
            serde_json::from_value::<AgentChainRow>(json!(row)).unwrap(),
            row
        );
        let mut no_price = row.clone();
        no_price.spend = ChainSpend::ViaGateway {
            tokens: 12_345,
            amount: None,
        };
        assert!(json!(no_price).get("amount").is_none());
        assert_eq!(
            serde_json::from_value::<AgentChainStatus>(json!("limit_reached")).unwrap(),
            AgentChainStatus::LimitReached
        );
        assert_eq!(
            serde_json::from_value::<AgentChainStatus>(json!("waking")).unwrap(),
            AgentChainStatus::Other
        );
        assert_eq!(
            serde_json::from_value::<ChainSpend>(json!({"type": "unknown"})).unwrap(),
            ChainSpend::Unknown
        );
        assert_eq!(
            serde_json::from_value::<ChainSpend>(json!({"type": "not_via_gateway"})).unwrap(),
            ChainSpend::NotViaGateway
        );
        assert_eq!(
            serde_json::from_value::<ChainSpend>(json!({"type": "via_gateway", "tokens": 5}))
                .unwrap(),
            ChainSpend::ViaGateway {
                tokens: 5,
                amount: None,
            }
        );
    }
}
