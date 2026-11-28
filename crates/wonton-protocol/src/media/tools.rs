//! 核心给 Agent 的媒体工具（ADR-0113 第 5 节）：工具名，和工具结果的失败体。参数见
//! `docs/adr/notes/0113-interfaces.md` 的工具表，由核心按 JSON Schema 发给 Agent，这里不另立类型。

use serde::{Deserialize, Serialize};

use super::known_or_other;

/// 查某个人或自己的任务与播放会话、进度、失败原因。只读，不发卡片。
pub const MEDIA_STATUS_TOOL: &str = "wonton__media_status";
/// 让一台设备立即跑一次备份。发卡片，发给本人。
pub const BACKUP_NOW_TOOL: &str = "wonton__backup_now";
/// 给一台设备排一个离线下载。发卡片，发给本人。
pub const MEDIA_DOWNLOAD_TOOL: &str = "wonton__media_download";
/// 在这个人正在用的设备上开播或接管。
pub const MEDIA_PLAY_TOOL: &str = "wonton__media_play";
/// 暂停、继续、跳转、上下首、音量。
pub const MEDIA_CONTROL_TOOL: &str = "wonton__media_control";
/// 读队列、改队列。
pub const MEDIA_QUEUE_TOOL: &str = "wonton__media_queue";
/// 列出这个人所有设备的会话与能力。
pub const MEDIA_SESSIONS_TOOL: &str = "wonton__media_sessions";
/// 暂停、继续、取消、重试一个任务。
pub const MEDIA_TASK_TOOL: &str = "wonton__media_task";
/// 删一台设备上的离线副本。发删除类卡片。
pub const MEDIA_DELETE_TOOL: &str = "wonton__media_delete";

/// 设备在等待上限内没回执命令时工具结果的码。它不是任务的失败，不进 [`super::MediaFailureCode`]。
pub const DEVICE_NO_RESPONSE: &str = "device_no_response";

/// 工具结果的失败体。任务、会话、提供方的失败照 [`super::MediaFailure`] 的码和原文；设备没回应时码是
/// [`DEVICE_NO_RESPONSE`]，`reasons` 列全可能的原因（ADR-0098 第 6 节），不挑一个：核心看不到是哪一种。
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct ToolFailureBody {
    pub code: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reasons: Vec<NoResponseReason>,
}

/// 设备没回应的可能原因。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NoResponseReason {
    PoweredOff,
    Offline,
    /// App 被从多任务里划掉了。
    AppClosed,
    /// 系统限了后台推送或后台执行。
    SystemThrottled,
    /// 新版本新加的原因，原值在里。
    Other(String),
}

known_or_other!(NoResponseReason {
    PoweredOff => "powered_off",
    Offline => "offline",
    AppClosed => "app_closed",
    SystemThrottled => "system_throttled",
});

impl NoResponseReason {
    /// 设备没回应时照列的全部原因。
    pub fn all() -> Vec<Self> {
        vec![
            Self::PoweredOff,
            Self::Offline,
            Self::AppClosed,
            Self::SystemThrottled,
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_device_that_did_not_answer_lists_every_possible_reason() {
        let body = ToolFailureBody {
            code: DEVICE_NO_RESPONSE.into(),
            message: "手机 30 秒没回应".into(),
            reasons: NoResponseReason::all(),
        };
        assert_eq!(
            serde_json::to_value(&body).unwrap()["reasons"],
            serde_json::json!(["powered_off", "offline", "app_closed", "system_throttled"])
        );
        let other: NoResponseReason =
            serde_json::from_value(serde_json::json!("low_power")).unwrap();
        assert_eq!(other, NoResponseReason::Other("low_power".into()));
    }

    #[test]
    fn a_task_failure_carries_no_reasons_field() {
        let body = ToolFailureBody {
            code: "media_gone".into(),
            message: "已不在库里".into(),
            reasons: Vec::new(),
        };
        assert_eq!(
            serde_json::to_value(&body).unwrap(),
            serde_json::json!({"code": "media_gone", "message": "已不在库里"})
        );
    }
}
