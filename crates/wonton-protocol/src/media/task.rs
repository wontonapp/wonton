use serde::ser::SerializeMap;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{MediaItemRef, Timestamp, known_or_other};

/// 任务号，由服务器核发。
pub type TaskId = Uuid;
/// 任务或会话的主人（MXID）。
pub type Person = String;
/// 设备标识。
pub type DeviceId = String;

/// 发一个新任务号。
pub fn new_task_id() -> TaskId {
    TaskId::new_v4()
}

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

/// 暂停的原因。新版本新加的原因落在 `Other`。
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum PauseReason {
    User,
    Agent,
    LowBattery,
    SuspendedBySystem,
    Other(String),
}

known_or_other!(PauseReason {
    User => "user",
    Agent => "agent",
    LowBattery => "low_battery",
    SuspendedBySystem => "suspended_by_system",
});

/// 失败的类别就是协议面，不是文案；壳按类别出字。
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct MediaFailure {
    pub code: MediaFailureCode,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry_after_secs: Option<u32>,
}

/// 失败类别（ADR-0113“失败类别”）：本地、传输、协议、上游、平台五组，码只增不改名。
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum MediaFailureCode {
    LocalSourceUnavailable,
    LocalPermissionDenied,
    LocalStorageFull,
    LocalIo,
    TransportUnavailable,
    Protocol,
    AuthenticationRequired,
    PermissionDenied,
    UnsupportedByDevice,
    RateLimited,
    MediaGone,
    PluginUnavailable,
    UpstreamError,
    ServerStorageFull,
    SuspendedBySystem,
    /// 新版本新加的失败码，原值在里；退避按不退避算。
    Other(String),
}

known_or_other!(MediaFailureCode {
    LocalSourceUnavailable => "local_source_unavailable",
    LocalPermissionDenied => "local_permission_denied",
    LocalStorageFull => "local_storage_full",
    LocalIo => "local_io",
    TransportUnavailable => "transport_unavailable",
    Protocol => "protocol",
    AuthenticationRequired => "authentication_required",
    PermissionDenied => "permission_denied",
    UnsupportedByDevice => "unsupported_by_device",
    RateLimited => "rate_limited",
    MediaGone => "media_gone",
    PluginUnavailable => "plugin_unavailable",
    UpstreamError => "upstream_error",
    ServerStorageFull => "server_storage_full",
    SuspendedBySystem => "suspended_by_system",
});

impl MediaFailureCode {
    /// 这类失败该等多久再试。认不得的码按不退避算，交给壳显示原文。
    pub fn backoff(&self) -> Backoff {
        match self {
            Self::LocalIo => Backoff::Once,
            Self::TransportUnavailable | Self::Protocol | Self::UpstreamError => Backoff::Standard,
            Self::PluginUnavailable => Backoff::ThreeTimes,
            Self::RateLimited => Backoff::RetryAfter,
            Self::SuspendedBySystem => Backoff::SystemPause,
            Self::Other(_)
            | Self::LocalSourceUnavailable
            | Self::LocalPermissionDenied
            | Self::LocalStorageFull
            | Self::AuthenticationRequired
            | Self::PermissionDenied
            | Self::UnsupportedByDevice
            | Self::MediaGone
            | Self::ServerStorageFull => Backoff::None,
        }
    }

    /// 这类失败是不是等等就可能自己好；`SystemPause` 与不退避的都算不是。
    pub fn backoff_is_retryable(&self) -> bool {
        matches!(
            self.backoff(),
            Backoff::Once | Backoff::Standard | Backoff::ThreeTimes | Backoff::RetryAfter
        )
    }
}

/// 一个失败类别的退避档。退避由核心算，不过壳。
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Backoff {
    /// 不退避：直接失败，或者跳过这一项。
    None,
    /// 1 秒一次；次数按任务共用 `retry_count`，别的码先退过就一次都退不了。
    Once,
    /// 1/2/4/8/16/32/60 秒，最多七次。
    Standard,
    /// 1/2/4 秒，最多三次。
    ThreeTimes,
    /// 按失败体里的 `retry_after_secs`（至少 1 秒），不计入七次。
    RetryAfter,
    /// 进 `Paused`，不算失败，恢复时从断点续。
    SystemPause,
}

/// 一条长任务。上传与下载共用；`target` 与 `progress.unit` 各自成立。
///
/// 带 `Media` 前缀：Swift 绑定进全局命名空间，`Task` 会和 Swift 自己的 `Task` 撞。
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct MediaTask {
    pub id: TaskId,
    pub kind: TaskKind,
    pub person: Person,
    pub device: DeviceId,
    pub target: TaskTarget,
    pub state: TaskState,
    pub progress: Progress,
    pub retry_count: u8,
    pub retry_of: Option<TaskId>,
    /// 下一次自动重试的秒级 Unix 时间；只在 `WaitingNetwork` 时有值，由核心算。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_attempt_at: Option<u64>,
    pub created_at: Timestamp,
    /// 只在上传里有；逐项失败原文
    pub partial_failures: Vec<FailedItem>,
}

/// 任务的目标。内部标记放不下序列，载荷另起 `value` 键；认不得的目标只留 `kind` 原文。
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum TaskTarget {
    /// 一批文件；相册备份 = 一个任务 + N 个 item
    Upload(Vec<UploadItem>),
    Download(MediaItemRef),
    /// 新版本新加的目标，原值在里，载荷丢弃。
    Other(String),
}

/// 认得的目标；未知 `kind` 落到 [`TaskTarget::Other`]。
#[derive(Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "value")]
enum KnownTaskTarget {
    Upload(Vec<UploadItem>),
    Download(MediaItemRef),
}

impl From<KnownTaskTarget> for TaskTarget {
    fn from(target: KnownTaskTarget) -> Self {
        match target {
            KnownTaskTarget::Upload(items) => Self::Upload(items),
            KnownTaskTarget::Download(item) => Self::Download(item),
        }
    }
}

impl Serialize for TaskTarget {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(2))?;
        match self {
            Self::Upload(items) => {
                map.serialize_entry("kind", "upload")?;
                map.serialize_entry("value", items)?;
            }
            Self::Download(item) => {
                map.serialize_entry("kind", "download")?;
                map.serialize_entry("value", item)?;
            }
            Self::Other(raw) => map.serialize_entry("kind", raw)?,
        }
        map.end()
    }
}

impl<'de> Deserialize<'de> for TaskTarget {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = serde_json::Value::deserialize(deserializer)?;
        let kind = value
            .get("kind")
            .and_then(|kind| kind.as_str())
            .map(str::to_owned)
            .ok_or_else(|| serde::de::Error::missing_field("kind"))?;
        match kind.as_str() {
            "upload" | "download" => KnownTaskTarget::deserialize(value)
                .map(Self::from)
                .map_err(serde::de::Error::custom),
            _ => Ok(Self::Other(kind)),
        }
    }
}

/// 上传的一项：路径、大小、校验和、拍摄时间，都是提供方要的。
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct UploadItem {
    pub item_id: String,
    pub path: String,
    pub size: u64,
    pub checksum: Option<String>,
    pub taken_at: Option<Timestamp>,
    pub state: ItemState,
    /// 服务端已知的已收字节；非 Done 项有效
    pub upload_offset: u64,
}

/// 逐项状态；`Partial` 由这里聚出来，不靠猜。认不得的状态把原值记在 `state` 里，当 `Other`。
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum ItemState {
    Queued,
    Running,
    Done,
    Skipped,
    Failed(MediaFailure),
    /// 新版本新加的状态，原值在里。
    Other(String),
}

impl ItemState {
    fn as_str(&self) -> &str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Done => "done",
            Self::Skipped => "skipped",
            Self::Failed(_) => "failed",
            Self::Other(raw) => raw,
        }
    }
}

impl Serialize for ItemState {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Failed(failure) => {
                let mut map = serializer
                    .serialize_map(Some(3 + usize::from(failure.retry_after_secs.is_some())))?;
                map.serialize_entry("state", "failed")?;
                map.serialize_entry("code", &failure.code)?;
                map.serialize_entry("message", &failure.message)?;
                if let Some(secs) = failure.retry_after_secs {
                    map.serialize_entry("retry_after_secs", &secs)?;
                }
                map.end()
            }
            known => {
                let mut map = serializer.serialize_map(Some(1))?;
                map.serialize_entry("state", known.as_str())?;
                map.end()
            }
        }
    }
}

impl<'de> Deserialize<'de> for ItemState {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let mut fields = serde_json::Map::deserialize(deserializer)?;
        let state = fields
            .remove("state")
            .ok_or_else(|| serde::de::Error::missing_field("state"))?;
        let state: String = serde_json::from_value(state).map_err(serde::de::Error::custom)?;
        Ok(match state.as_str() {
            "queued" => Self::Queued,
            "running" => Self::Running,
            "done" => Self::Done,
            "skipped" => Self::Skipped,
            "failed" => Self::Failed(
                serde_json::from_value(serde_json::Value::Object(fields))
                    .map_err(serde::de::Error::custom)?,
            ),
            _ => Self::Other(state),
        })
    }
}

/// 任务里的一项是怎么失败的。
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct FailedItem {
    pub item_id: String,
    pub path: String,
    pub failure: MediaFailure,
}

/// 进度；`unit` 是字节还是项，两边不混用。
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct Progress {
    pub done: u64,
    pub total: Option<u64>,
    pub unit: ProgressUnit,
}

/// 进度的单位。新版本新加的单位落在 `Other`。
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum ProgressUnit {
    Bytes,
    Items,
    /// 新版本新加的单位，原值在里。
    Other(String),
}

known_or_other!(ProgressUnit {
    Bytes => "bytes",
    Items => "items",
});

/// 建任务用的入参。
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct TaskCreate {
    pub kind: TaskKind,
    pub person: Person,
    pub device: DeviceId,
    pub target: TaskTarget,
    pub options: TaskOptions,
}

fn one() -> u8 {
    1
}

/// 建任务时各轴自己的开关（只在传输、只在充电、转码率上限）。
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct TaskOptions {
    pub wifi_only: bool,
    pub charging_only: bool,
    pub max_bitrate: Option<u32>,
    /// 相册范围（可选）；只传相册不传 items 时由设备枚举
    pub albums: Vec<String>,
    /// 同时在跑几项，1–4
    #[serde(default = "one")]
    pub concurrency: u8,
    /// 立即备份：忽略 wifi/充电条件一次
    pub force_now: bool,
}

impl Default for TaskOptions {
    fn default() -> Self {
        Self {
            wifi_only: false,
            charging_only: false,
            max_bitrate: None,
            albums: Vec::new(),
            concurrency: one(),
            force_now: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn failure(code: MediaFailureCode) -> MediaFailure {
        MediaFailure {
            code,
            message: "炸了".into(),
            retry_after_secs: None,
        }
    }

    fn queued() -> TaskState {
        TaskState::Queued
    }

    fn preparing() -> TaskState {
        TaskState::Preparing
    }

    fn running() -> TaskState {
        TaskState::Running
    }

    fn paused() -> TaskState {
        TaskState::Paused(PauseReason::User)
    }

    fn waiting_network() -> TaskState {
        TaskState::WaitingNetwork
    }

    fn waiting_power() -> TaskState {
        TaskState::WaitingPower
    }

    #[test]
    fn legal_transitions_pass() {
        let legal = [
            (queued(), preparing()),
            (queued(), TaskState::Cancelled),
            (preparing(), running()),
            (
                preparing(),
                TaskState::Failed(failure(MediaFailureCode::Protocol)),
            ),
            (preparing(), TaskState::Cancelled),
            (running(), paused()),
            (running(), waiting_network()),
            (running(), waiting_power()),
            (running(), TaskState::Done),
            (running(), TaskState::Partial),
            (
                running(),
                TaskState::Failed(failure(MediaFailureCode::UpstreamError)),
            ),
            (running(), TaskState::Cancelled),
            (preparing(), TaskState::Done),
            (preparing(), TaskState::Partial),
            (paused(), running()),
            (paused(), TaskState::Paused(PauseReason::Agent)),
            (paused(), TaskState::Cancelled),
            (paused(), queued()),
            (waiting_network(), running()),
            (waiting_network(), paused()),
            (waiting_network(), queued()),
            (waiting_network(), waiting_network()),
            (
                waiting_network(),
                TaskState::Failed(failure(MediaFailureCode::TransportUnavailable)),
            ),
            (waiting_network(), TaskState::Cancelled),
            (waiting_power(), running()),
            (waiting_power(), queued()),
            (waiting_power(), waiting_power()),
            (waiting_power(), paused()),
            (waiting_power(), TaskState::Cancelled),
        ];
        for (from, to) in legal {
            assert_eq!(
                from.clone().transition(to.clone()),
                Ok(to.clone()),
                "{from:?}"
            );
            assert_eq!(TaskState::try_from((from, to.clone())), Ok(to));
        }
    }

    #[test]
    fn terminal_states_never_go_back_to_running() {
        let terminals = [
            TaskState::Done,
            TaskState::Partial,
            TaskState::Failed(failure(MediaFailureCode::MediaGone)),
            TaskState::Cancelled,
        ];
        let targets = [
            queued(),
            preparing(),
            running(),
            paused(),
            waiting_network(),
            waiting_power(),
        ];
        for terminal in terminals {
            for target in &targets {
                assert!(
                    terminal.clone().transition(target.clone()).is_err(),
                    "{terminal:?} → {target:?}"
                );
            }
        }
    }

    #[test]
    fn illegal_transition_names_both_states() {
        let error = TaskState::Done.transition(TaskState::Running).unwrap_err();
        assert_eq!(error.from, TaskState::Done);
        assert_eq!(error.to, TaskState::Running);
        assert_eq!(error.to_string(), "任务状态不能从 done 变到 running");
    }

    #[test]
    fn every_failure_code_has_its_backoff() {
        let expected = [
            (MediaFailureCode::LocalSourceUnavailable, Backoff::None),
            (MediaFailureCode::LocalPermissionDenied, Backoff::None),
            (MediaFailureCode::LocalStorageFull, Backoff::None),
            (MediaFailureCode::LocalIo, Backoff::Once),
            (MediaFailureCode::TransportUnavailable, Backoff::Standard),
            (MediaFailureCode::Protocol, Backoff::Standard),
            (MediaFailureCode::AuthenticationRequired, Backoff::None),
            (MediaFailureCode::PermissionDenied, Backoff::None),
            (MediaFailureCode::UnsupportedByDevice, Backoff::None),
            (MediaFailureCode::RateLimited, Backoff::RetryAfter),
            (MediaFailureCode::MediaGone, Backoff::None),
            (MediaFailureCode::PluginUnavailable, Backoff::ThreeTimes),
            (MediaFailureCode::UpstreamError, Backoff::Standard),
            (MediaFailureCode::ServerStorageFull, Backoff::None),
            (MediaFailureCode::SuspendedBySystem, Backoff::SystemPause),
            (MediaFailureCode::Other("quantum".into()), Backoff::None),
        ];
        assert_eq!(expected.len(), 16);
        for (code, backoff) in expected {
            assert_eq!(code.backoff(), backoff, "{code:?}");
        }
    }

    #[test]
    fn failures_without_retry_after_omit_the_field() {
        let json = serde_json::to_value(failure(MediaFailureCode::RateLimited)).unwrap();
        assert_eq!(json["code"], "rate_limited");
        assert!(json.get("retry_after_secs").is_none());

        let limited = MediaFailure {
            retry_after_secs: Some(30),
            ..failure(MediaFailureCode::RateLimited)
        };
        let json = serde_json::to_value(&limited).unwrap();
        assert_eq!(json["retry_after_secs"], 30);
    }

    #[test]
    fn task_and_targets_roundtrip() {
        let task = MediaTask {
            id: Uuid::nil(),
            kind: TaskKind::Upload,
            person: "Alice".into(),
            device: "phone".into(),
            target: TaskTarget::Upload(vec![UploadItem {
                item_id: "i1".into(),
                path: "DCIM/a.jpg".into(),
                size: 2048,
                checksum: Some("sha256:x".into()),
                taken_at: Some(1_700_000_000_000),
                state: ItemState::Failed(failure(MediaFailureCode::LocalIo)),
                upload_offset: 512,
            }]),
            state: TaskState::Partial,
            progress: Progress {
                done: 1,
                total: Some(2),
                unit: ProgressUnit::Items,
            },
            retry_count: 2,
            retry_of: Some(Uuid::nil()),
            next_attempt_at: Some(1_700_000_010),
            created_at: 1_700_000_000_000,
            partial_failures: vec![FailedItem {
                item_id: "i1".into(),
                path: "DCIM/a.jpg".into(),
                failure: failure(MediaFailureCode::LocalIo),
            }],
        };
        let json = serde_json::to_value(&task).unwrap();
        let back: MediaTask = serde_json::from_value(json.clone()).unwrap();
        assert_eq!(serde_json::to_value(&back).unwrap(), json);

        let download = TaskTarget::Download(super::super::tests::item());
        let json = serde_json::to_value(&download).unwrap();
        assert_eq!(json["kind"], "download");
        let back: TaskTarget = serde_json::from_value(json.clone()).unwrap();
        assert_eq!(serde_json::to_value(&back).unwrap(), json);
    }

    #[test]
    fn item_state_failure_inlines_the_tag() {
        let json =
            serde_json::to_value(ItemState::Failed(failure(MediaFailureCode::MediaGone))).unwrap();
        assert_eq!(json["state"], "failed");
        assert_eq!(json["code"], "media_gone");
        let back: ItemState = serde_json::from_value(json.clone()).unwrap();
        assert_eq!(serde_json::to_value(&back).unwrap(), json);
    }

    #[test]
    fn unknown_task_target_and_progress_unit_fall_to_other() {
        let mut json = serde_json::to_value(MediaTask {
            id: Uuid::nil(),
            kind: TaskKind::Upload,
            person: "Alice".into(),
            device: "phone".into(),
            target: TaskTarget::Download(super::super::tests::item()),
            state: TaskState::Running,
            progress: Progress {
                done: 1,
                total: Some(2),
                unit: ProgressUnit::Items,
            },
            retry_count: 0,
            retry_of: None,
            next_attempt_at: None,
            created_at: 1_700_000_000_000,
            partial_failures: Vec::new(),
        })
        .unwrap();
        json["target"] = serde_json::json!({"kind": "stream"});
        json["progress"]["unit"] = serde_json::json!("chunks");

        let back: MediaTask = serde_json::from_value(json.clone()).unwrap();
        assert_eq!(back.target, TaskTarget::Other("stream".into()));
        assert_eq!(back.progress.unit, ProgressUnit::Other("chunks".into()));
        assert_eq!(serde_json::to_value(&back).unwrap(), json);
    }

    #[test]
    fn unknown_task_values_fall_to_other() {
        let kind: TaskKind = serde_json::from_str("\"migrate\"").unwrap();
        assert_eq!(kind, TaskKind::Other("migrate".into()));
        assert_eq!(
            serde_json::to_value(&kind).unwrap(),
            serde_json::json!("migrate")
        );

        let code: MediaFailureCode = serde_json::from_str("\"quantum\"").unwrap();
        assert_eq!(code, MediaFailureCode::Other("quantum".into()));
        assert_eq!(
            serde_json::to_value(&code).unwrap(),
            serde_json::json!("quantum")
        );

        let state: TaskState = serde_json::from_str("\"melting\"").unwrap();
        assert_eq!(state, TaskState::Other("melting".into()));
        assert_eq!(state.as_str(), "melting");
        assert_eq!(
            serde_json::to_value(&state).unwrap(),
            serde_json::json!("melting")
        );

        let item: ItemState =
            serde_json::from_str("{\"state\": \"frozen\", \"since\": 3}").unwrap();
        assert_eq!(item, ItemState::Other("frozen".into()));
        assert_eq!(
            serde_json::to_value(&item).unwrap(),
            serde_json::json!({"state": "frozen"})
        );
    }

    #[test]
    fn unknown_tagged_task_state_keeps_the_tag_name() {
        let state: TaskState = serde_json::from_str("{\"migrating\": {\"a\": 1}}").unwrap();
        assert_eq!(state, TaskState::Other("migrating".into()));
        assert_eq!(
            serde_json::to_value(&state).unwrap(),
            serde_json::json!("migrating")
        );
    }

    #[test]
    fn unknown_states_only_enter_and_leave_as_themselves() {
        let migrating = TaskState::Other("migrating".into());
        let paused = TaskState::Other("paused".into());
        assert_eq!(
            TaskState::Running.transition(migrating.clone()),
            Ok(migrating.clone())
        );
        assert_eq!(
            TaskState::Preparing.transition(migrating.clone()),
            Ok(migrating.clone())
        );
        assert_eq!(migrating.clone().transition(paused.clone()), Ok(paused));
        assert!(TaskState::Done.transition(migrating.clone()).is_err());
        assert!(TaskState::Cancelled.transition(migrating.clone()).is_err());
        assert!(migrating.transition(TaskState::Running).is_err());
    }

    #[test]
    fn out_of_order_known_transitions_are_errors() {
        assert!(TaskState::Queued.transition(TaskState::Running).is_err());
        assert!(TaskState::Running.transition(TaskState::Preparing).is_err());
    }

    #[test]
    fn default_task_options_use_one_concurrency() {
        let options = TaskOptions::default();
        assert_eq!(options.concurrency, 1);
        assert!(!options.wifi_only);
        assert!(options.albums.is_empty());
    }

    #[test]
    fn task_options_default_to_one_concurrency() {
        let options: TaskOptions = serde_json::from_value(serde_json::json!({
            "wifi_only": false,
            "charging_only": false,
            "max_bitrate": null,
            "albums": [],
            "force_now": false
        }))
        .unwrap();
        assert_eq!(options.concurrency, 1);
        assert!(!options.wifi_only);
        assert!(!options.charging_only);
        assert!(options.albums.is_empty());
        assert!(!options.force_now);
    }
}
