//! Bot 一轮里调过的工具（ADR-0128）：状态消息旁边的 `app.wonton.trace` 清单，每项是一次工具调用。
//! 字段和取值照 ACP 的 `ToolCall`（`kind` 是 `ToolKind`，`status` 是 `ToolCallStatus`），是发送者的自述。

use serde::{Deserialize, Serialize};

/// 字符串取值的枚举：认识的取值各是一个变体，不认识的落到 `Other` 并带着原值，往返不丢。
macro_rules! wire_enum {
    ($name:ident { $($variant:ident => $raw:literal),+ $(,)? }) => {
        impl $name {
            fn known(raw: &str) -> Option<Self> {
                match raw {
                    $($raw => Some(Self::$variant),)+
                    _ => None,
                }
            }
        }

        impl Serialize for $name {
            fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
            where
                S: serde::Serializer,
            {
                serializer.serialize_str(match self {
                    $(Self::$variant => $raw,)+
                    Self::Other(raw) => raw,
                })
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: serde::Deserializer<'de>,
            {
                let raw = String::deserialize(deserializer)?;
                Ok(Self::known(&raw).unwrap_or(Self::Other(raw)))
            }
        }
    };
}

/// 这次调用是哪一类（ACP 的 `ToolKind`）：壳按它挑图标。
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum AgentToolKind {
    /// 读文件、看网页快照、看图。
    Read,
    /// 写文件、改文件。
    Edit,
    Delete,
    Move,
    /// 搜文件。
    Search,
    /// 执行命令、跑代码、操作浏览器、委派。
    Execute,
    /// 在想。
    Think,
    /// 上网：搜索、取网页。
    Fetch,
    /// ACP 的 `other`、`switch_mode` 和以后新加的取值，原值带着；壳画通用图标。
    Other(String),
}

wire_enum!(AgentToolKind {
    Read => "read",
    Edit => "edit",
    Delete => "delete",
    Move => "move",
    Search => "search",
    Execute => "execute",
    Think => "think",
    Fetch => "fetch",
});

/// 这次调用到哪了（ACP 的 `ToolCallStatus`）。`Uncertain` 由客户端核心合成：一轮已经结束或显示成不确定时，
/// 还标着进行中的项按它显示，不转圈（ADR-0128 第 4 节）。
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum AgentToolStatus {
    InProgress,
    Completed,
    Failed,
    /// 结果不确定，核心合成。
    Uncertain,
    /// ACP 的 `pending` 和以后新加的取值，原值带着；壳按进行中以外的普通一行画，不加标记。
    Other(String),
}

wire_enum!(AgentToolStatus {
    InProgress => "in_progress",
    Completed => "completed",
    Failed => "failed",
    Uncertain => "uncertain",
});

/// 一次工具调用。`title` 是 Bot 写的一行说明（私聊里是参数预览，多人房间里只写类别，ADR-0128 第 3 节），原样显示，不翻译。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct AgentToolCall {
    /// 这次调用的 ID（ACP 的 `toolCallId`），同一轮里唯一。
    pub id: String,
    pub kind: AgentToolKind,
    pub title: String,
    pub status: AgentToolStatus,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 契约：类别的每个取值都能经 JSON 往返；线上是新加的取值（包括 ACP 的 `other`）时落到 `Other`，原样带回来。
    #[test]
    fn kinds_round_trip_and_unknown_ones_carry_their_raw_value() {
        for (kind, json) in [
            (AgentToolKind::Read, "read"),
            (AgentToolKind::Edit, "edit"),
            (AgentToolKind::Delete, "delete"),
            (AgentToolKind::Move, "move"),
            (AgentToolKind::Search, "search"),
            (AgentToolKind::Execute, "execute"),
            (AgentToolKind::Think, "think"),
            (AgentToolKind::Fetch, "fetch"),
            (AgentToolKind::Other("other".into()), "other"),
            (AgentToolKind::Other("switch_mode".into()), "switch_mode"),
        ] {
            assert_eq!(
                serde_json::to_value(&kind).unwrap(),
                serde_json::json!(json)
            );
            assert_eq!(
                serde_json::from_value::<AgentToolKind>(serde_json::json!(json)).unwrap(),
                kind
            );
        }
    }

    /// 契约：状态的每个取值都能经 JSON 往返；线上是新加的取值时落到 `Other`，原样带回来。
    #[test]
    fn statuses_round_trip_and_unknown_ones_carry_their_raw_value() {
        for (status, json) in [
            (AgentToolStatus::InProgress, "in_progress"),
            (AgentToolStatus::Completed, "completed"),
            (AgentToolStatus::Failed, "failed"),
            (AgentToolStatus::Uncertain, "uncertain"),
            (AgentToolStatus::Other("pending".into()), "pending"),
        ] {
            assert_eq!(
                serde_json::to_value(&status).unwrap(),
                serde_json::json!(json)
            );
            assert_eq!(
                serde_json::from_value::<AgentToolStatus>(serde_json::json!(json)).unwrap(),
                status
            );
        }
    }

    /// 契约：一项调用的字段按蛇形命名，四个都在。
    #[test]
    fn a_tool_call_has_a_stable_json_contract() {
        let call = AgentToolCall {
            id: "call_1".into(),
            kind: AgentToolKind::Execute,
            title: "terminal: ls".into(),
            status: AgentToolStatus::InProgress,
        };
        let json = serde_json::to_value(&call).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "id": "call_1",
                "kind": "execute",
                "title": "terminal: ls",
                "status": "in_progress",
            })
        );
        assert_eq!(serde_json::from_value::<AgentToolCall>(json).unwrap(), call);
    }
}
