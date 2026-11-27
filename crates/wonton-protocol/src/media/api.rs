//! 核心的媒体接口 `/_wonton/api/media/…`（客户端核 → 服务器核）：路径和请求体、响应体。
//!
//! 只接经隧道的已配对设备；人由核心按设备记录认，请求里写的 `person` 必须和它相同，不同按
//! `permission_denied` 拒。错误体一律是 [`MediaFailure`] 信封；只有队列的 409 回当前 [`Queue`](super::Queue)。
//! 请求另带 [`super::WONTON_DEVICE_HEADER`]，同一个人两台设备时用它区分。
//!
//! 字节走三条路由（上传、取内容、取流），核心流式转给提供方、不落盘；`Range`、`If-Range`、
//! `Content-Range` 原样过。

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::provider::{kind_param, segment};
use super::session::{PlaybackSession, PlaybackSessionId, Revision};
use super::task::{MediaFailure, MediaTask, PauseReason, TaskId, TaskKind};
use super::view::{DeviceCommand, QueueOp};
use super::{Timestamp, known_or_other};

pub const MEDIA_API_PREFIX: &str = "/_wonton/api/media";

pub fn tasks_path() -> String {
    format!("{MEDIA_API_PREFIX}/tasks")
}

pub fn task_path(task: TaskId) -> String {
    format!("{MEDIA_API_PREFIX}/tasks/{task}")
}

/// `POST /tasks/{id}/{pause|resume|cancel|retry}`
pub fn task_action_path(task: TaskId, action: TaskAction) -> String {
    format!("{}/{}", task_path(task), action.as_str())
}

/// `POST|PATCH /tasks/{id}/uploads/{item}`，`item` 是 `UploadItem.item_id`。
pub fn task_upload_path(task: TaskId, item: &str) -> String {
    format!("{}/uploads/{}", task_path(task), segment(item))
}

/// `GET /tasks/{id}/items/{item}/content`：下载任务的字节，`item` 是 `MediaItemRef.id`。
pub fn task_item_content_path(task: TaskId, item: &str) -> String {
    format!("{}/items/{}/content", task_path(task), segment(item))
}

/// `POST /tasks/{id}/progress`：下载的进度由设备报，体是 [`TaskProgressReport`]；上传的由核心按偏移算。
pub fn task_progress_path(task: TaskId) -> String {
    format!("{}/progress", task_path(task))
}

/// `POST /tasks/{id}/complete`：下载写完、改好名了。
pub fn task_complete_path(task: TaskId) -> String {
    format!("{}/complete", task_path(task))
}

/// `POST /tasks/{id}/failure`：设备那边的失败（手机空间不足、没权限……），体是 [`MediaFailure`]；
/// 退避与否由核心按类别定，回更新后的任务。
pub fn task_failure_path(task: TaskId) -> String {
    format!("{}/failure", task_path(task))
}

/// `POST /tasks/{id}/items/{item}/failure`：上传的一项在设备那边失败（源文件不见了），这一项标失败、
/// 其余照常；体是 [`MediaFailure`]。
pub fn task_item_failure_path(task: TaskId, item: &str) -> String {
    format!("{}/items/{}/failure", task_path(task), segment(item))
}

pub fn sessions_path() -> String {
    format!("{MEDIA_API_PREFIX}/sessions")
}

pub fn session_path(session: PlaybackSessionId) -> String {
    format!("{MEDIA_API_PREFIX}/sessions/{session}")
}

pub fn session_control_path(session: PlaybackSessionId) -> String {
    format!("{}/control", session_path(session))
}

pub fn session_progress_path(session: PlaybackSessionId) -> String {
    format!("{}/progress", session_path(session))
}

pub fn session_stop_path(session: PlaybackSessionId) -> String {
    format!("{}/stop", session_path(session))
}

/// `GET /sessions/{id}/stream`：当前计划的流，核心转给提供方的流地址。
pub fn session_stream_path(session: PlaybackSessionId) -> String {
    format!("{}/stream", session_path(session))
}

pub fn session_cast_url_path(session: PlaybackSessionId) -> String {
    format!("{}/cast-url", session_path(session))
}

pub fn queue_path() -> String {
    format!("{MEDIA_API_PREFIX}/queue")
}

/// `GET /items?…`：按种类列条目，核心转给这个种类的提供方；查询参数同提供方的 `GET /items`。
pub fn items_path() -> String {
    format!("{MEDIA_API_PREFIX}/items")
}

/// `GET /items/{id}/artwork?kind=…`：封面、海报、缩略图的字节。
pub fn item_artwork_path(item: &str, kind: &super::MediaKind) -> String {
    format!(
        "{MEDIA_API_PREFIX}/items/{}/artwork?kind={}",
        segment(item),
        segment(kind_param(kind))
    )
}

pub fn commands_path() -> String {
    format!("{MEDIA_API_PREFIX}/commands")
}

pub fn command_ack_path(command: Uuid) -> String {
    format!("{MEDIA_API_PREFIX}/commands/{command}/ack")
}

/// 任务上能做的动作。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskAction {
    Pause,
    Resume,
    Cancel,
    Retry,
}

impl TaskAction {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pause => "pause",
            Self::Resume => "resume",
            Self::Cancel => "cancel",
            Self::Retry => "retry",
        }
    }
}

/// `GET /tasks` 的查询参数。
#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq)]
pub struct TaskQuery {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub person: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<TaskKind>,
    /// `TaskState` 的标签名（`queued`、`failed`……）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct TaskList {
    pub tasks: Vec<MediaTask>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
}

/// 设备报的下载进度：已写到本机的字节，知道总数时带上。
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub struct TaskProgressReport {
    pub done: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total: Option<u64>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct PauseRequest {
    pub reason: PauseReason,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct SessionList {
    pub sessions: Vec<PlaybackSession>,
}

/// `POST /queue` 的请求体：读到的版本和这次的改动。版本不等回 409 与当前 [`Queue`](super::Queue)。
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct QueueChange {
    pub revision: Revision,
    pub op: QueueOp,
}

/// 投屏的接收端。新版本新加的落在 `Other`。
#[derive(Debug, Clone, PartialEq)]
pub enum CastTarget {
    Airplay,
    Cast,
    Other(String),
}

known_or_other!(CastTarget {
    Airplay => "airplay",
    Cast => "cast",
});

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct CastRequest {
    pub target: CastTarget,
}

/// 绑局域网地址的一次性绝对地址，只对这一项有效。
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct CastUrl {
    pub url: String,
    pub expires_at: Timestamp,
}

/// `GET /commands` 的查询参数：设备只拉自己的，`after` 是上次回执过的最大 `seq`。
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct CommandQuery {
    pub device: String,
    #[serde(default)]
    pub after: u64,
}

/// 长轮询的响应；最长挂 25 秒，没有命令回 204、不带体。
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct CommandList {
    pub commands: Vec<DeviceCommand>,
}

/// `POST /commands/{id}/ack` 的请求体；重复回执没有副作用。
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct CommandAck {
    pub seq: u64,
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure: Option<MediaFailure>,
}

/// `GET /items` 的响应，同提供方的 [`super::provider::ItemsPage`]。
pub type ItemsPage = super::provider::ItemsPage;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media::MediaKind;

    #[test]
    fn routes_live_under_the_media_prefix() {
        let task = Uuid::nil();
        assert_eq!(
            task_action_path(task, TaskAction::Retry),
            "/_wonton/api/media/tasks/00000000-0000-0000-0000-000000000000/retry"
        );
        assert_eq!(
            task_upload_path(task, "IMG 1"),
            "/_wonton/api/media/tasks/00000000-0000-0000-0000-000000000000/uploads/IMG%201"
        );
        assert_eq!(
            task_item_content_path(task, "a/b"),
            "/_wonton/api/media/tasks/00000000-0000-0000-0000-000000000000/items/a%2Fb/content"
        );
        assert_eq!(
            session_stream_path(task),
            "/_wonton/api/media/sessions/00000000-0000-0000-0000-000000000000/stream"
        );
        assert_eq!(
            item_artwork_path("al 1", &MediaKind::Audio),
            "/_wonton/api/media/items/al%201/artwork?kind=audio"
        );
        assert_eq!(queue_path(), "/_wonton/api/media/queue");
        assert_eq!(
            command_ack_path(task),
            "/_wonton/api/media/commands/00000000-0000-0000-0000-000000000000/ack"
        );
    }

    #[test]
    fn unknown_cast_target_is_kept() {
        let request: CastRequest =
            serde_json::from_value(serde_json::json!({"target": "miracast"})).unwrap();
        assert_eq!(request.target, CastTarget::Other("miracast".into()));
        assert_eq!(
            serde_json::to_value(&request).unwrap(),
            serde_json::json!({"target": "miracast"})
        );
    }

    #[test]
    fn command_query_defaults_after_to_zero() {
        let query: CommandQuery =
            serde_json::from_value(serde_json::json!({"device": "d1"})).unwrap();
        assert_eq!(query.after, 0);
    }
}
