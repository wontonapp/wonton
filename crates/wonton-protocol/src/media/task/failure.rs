//! 暂停原因、失败类别和退避档。

use serde::{Deserialize, Serialize};

use super::super::known_or_other;

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
    /// 能做这件事的媒体源不止一家，主人又没定默认：App 请用户选一家，带 `source` 再来。
    SourceRequired,
    /// 队列被别的设备或 Agent 先改了，这次按位置的改动对不上了：客户端核已换上当前队列，请用户看过再改。
    StaleRevision,
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
    SourceRequired => "source_required",
    StaleRevision => "stale_revision",
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
            | Self::ServerStorageFull
            | Self::SourceRequired
            | Self::StaleRevision => Backoff::None,
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
