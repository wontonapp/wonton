//! 媒体库、任务和会话服务报的错折成失败信封的码。设备接口和 Agent 工具共用这一份，同一个错在两边是同一个码。

use wonton_protocol::media::{MediaFailure, MediaFailureCode};

use crate::plugin::media_source::failure;

/// 媒体库、状态机报的错折成信封：找不到的当 `media_gone`，不合法的转移当 `protocol`，其余是核心自己出错。
pub(crate) fn service_failure(error: &dyn ServiceError) -> MediaFailure {
    failure(error.code(), error.to_string())
}

pub(crate) trait ServiceError: std::fmt::Display {
    fn code(&self) -> MediaFailureCode;
}

impl ServiceError for crate::media::MediaStoreError {
    fn code(&self) -> MediaFailureCode {
        use crate::media::MediaStoreError as E;
        match self {
            E::TaskNotFound { .. } | E::SessionNotFound { .. } => MediaFailureCode::MediaGone,
            E::TaskNotUpload { .. } => MediaFailureCode::Protocol,
            E::QuotaExceeded { ledger, .. } if *ledger == "device_offline" => {
                MediaFailureCode::LocalStorageFull
            }
            E::QuotaExceeded { .. } => MediaFailureCode::ServerStorageFull,
            _ => MediaFailureCode::UpstreamError,
        }
    }
}

impl ServiceError for crate::media::TaskServiceError {
    fn code(&self) -> MediaFailureCode {
        use crate::media::TaskServiceError as E;
        match self {
            E::Store(store) => store.code(),
            E::Transition(_) | E::NotUpload { .. } => MediaFailureCode::Protocol,
            E::ItemNotFound { .. } => MediaFailureCode::MediaGone,
            E::ConcurrentChange { .. } => MediaFailureCode::UpstreamError,
            E::Refused(failure) => failure.code.clone(),
        }
    }
}

impl ServiceError for crate::media::commands::CommandError {
    fn code(&self) -> MediaFailureCode {
        use crate::media::commands::CommandError as E;
        match self {
            E::Store(store) => store.code(),
            E::NotThisDevice(_) => MediaFailureCode::PermissionDenied,
            E::SeqMismatch { .. } => MediaFailureCode::Protocol,
        }
    }
}

impl ServiceError for crate::media::session_service::SessionServiceError {
    fn code(&self) -> MediaFailureCode {
        use crate::media::session_service::SessionServiceError as E;
        match self {
            E::Store(store) => store.code(),
            E::Transition(_) | E::UnsupportedControl(_) | E::QueueIndex { .. } => {
                MediaFailureCode::Protocol
            }
            E::ConcurrentChange { .. } => MediaFailureCode::UpstreamError,
        }
    }
}
