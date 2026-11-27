//! 任务类型、任务状态和合法转移。

use serde::{Deserialize, Serialize};

use super::super::known_or_other;
use super::failure::{MediaFailure, PauseReason};

/// 上传与下载。新版本新加的类型落在 `Other`。
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum TaskKind {
    Upload,
    Download,
    /// 新版本新加的类型，原值在里。
    Other(String),
}

known_or_other!(TaskKind {
    Upload => "upload",
    Download => "download",
});

/// 一条长任务的状态，上传、下载和播放会话共用一份。
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum TaskState {
    Queued,
    Preparing,
    Running,
    Paused(PauseReason),
    WaitingNetwork,
    WaitingPower,
    Done,
    Partial,
    Failed(MediaFailure),
    Cancelled,
    /// 新版本新加的状态，原值在里；旧核不知道它该往哪走。
    Other(String),
}

impl TaskState {
    fn known(raw: &str) -> Option<Self> {
        Some(match raw {
            "queued" => Self::Queued,
            "preparing" => Self::Preparing,
            "running" => Self::Running,
            "waiting_network" => Self::WaitingNetwork,
            "waiting_power" => Self::WaitingPower,
            "done" => Self::Done,
            "partial" => Self::Partial,
            "cancelled" => Self::Cancelled,
            _ => return None,
        })
    }

    /// 序列化用的名字，报错和日志里也用它。
    pub fn as_str(&self) -> &str {
        match self {
            Self::Queued => "queued",
            Self::Preparing => "preparing",
            Self::Running => "running",
            Self::Paused(_) => "paused",
            Self::WaitingNetwork => "waiting_network",
            Self::WaitingPower => "waiting_power",
            Self::Done => "done",
            Self::Partial => "partial",
            Self::Failed(_) => "failed",
            Self::Cancelled => "cancelled",
            Self::Other(raw) => raw,
        }
    }

    /// 终态（`Done`/`Partial`/`Failed`/`Cancelled`），不可逆。
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::Done | Self::Partial | Self::Failed(_) | Self::Cancelled
        )
    }

    /// 能不能从自己变到 `to`。排队和准备阶段可以直接取消（`Queued` 也行，见接口清单）；等待条件和暂停中
    /// 可以回队、互换原因，也能直接失败；终态不可逆，再跑就是新任务。未知状态（`Other`）的规则只有服务端
    /// 知道：可以进它，从它出来算非法。
    pub fn transition(self, to: Self) -> Result<Self, IllegalTransition> {
        let legal = match (&self, &to) {
            (Self::Other(_), Self::Other(_)) => true,
            (Self::Other(_), _) => false,
            (_, Self::Other(_)) => !self.is_terminal(),
            _ => matches!(
                (&self, &to),
                (Self::Queued, Self::Preparing | Self::Cancelled)
                    | (
                        Self::Preparing,
                        Self::Running
                            | Self::Done
                            | Self::Partial
                            | Self::Failed(_)
                            | Self::Cancelled
                    )
                    | (
                        Self::Running,
                        Self::Paused(_)
                            | Self::WaitingNetwork
                            | Self::WaitingPower
                            | Self::Done
                            | Self::Partial
                            | Self::Failed(_)
                            | Self::Cancelled
                    )
                    | (
                        Self::Paused(_) | Self::WaitingNetwork | Self::WaitingPower,
                        Self::Queued
                            | Self::Running
                            | Self::Paused(_)
                            | Self::WaitingNetwork
                            | Self::WaitingPower
                            | Self::Failed(_)
                            | Self::Cancelled
                    )
            ),
        };
        if legal {
            Ok(to)
        } else {
            Err(IllegalTransition { from: self, to })
        }
    }
}

impl Serialize for TaskState {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Paused(reason) => {
                serializer.serialize_newtype_variant("TaskState", 3, "paused", reason)
            }
            Self::Failed(failure) => {
                serializer.serialize_newtype_variant("TaskState", 8, "failed", failure)
            }
            known => serializer.serialize_str(known.as_str()),
        }
    }
}

impl<'de> Deserialize<'de> for TaskState {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = serde_json::Value::deserialize(deserializer)?;
        match value {
            serde_json::Value::String(raw) => Ok(Self::known(&raw).unwrap_or(Self::Other(raw))),
            serde_json::Value::Object(fields) if fields.len() == 1 => {
                let (key, payload) = fields.into_iter().next().expect("刚判过只有一个键");
                match key.as_str() {
                    "paused" => Ok(Self::Paused(
                        serde_json::from_value(payload).map_err(serde::de::Error::custom)?,
                    )),
                    "failed" => Ok(Self::Failed(
                        serde_json::from_value(payload).map_err(serde::de::Error::custom)?,
                    )),
                    _ => Ok(Self::Other(key)),
                }
            }
            other => Err(serde::de::Error::custom(format!(
                "任务状态应该是字符串或单键对象，拿到 {other}"
            ))),
        }
    }
}

/// 「从，到」两态直接转，规则同 [`TaskState::transition`]。
impl TryFrom<(TaskState, TaskState)> for TaskState {
    type Error = IllegalTransition;

    fn try_from((from, to): (TaskState, TaskState)) -> Result<Self, Self::Error> {
        from.transition(to)
    }
}

/// 状态机不允许的转移。只在 Rust 的 `TryFrom` 里用，不过壳。
#[derive(Debug, Clone, PartialEq)]
pub struct IllegalTransition {
    pub from: TaskState,
    pub to: TaskState,
}

impl std::fmt::Display for IllegalTransition {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "任务状态不能从 {} 变到 {}",
            self.from.as_str(),
            self.to.as_str()
        )
    }
}

impl std::error::Error for IllegalTransition {}
