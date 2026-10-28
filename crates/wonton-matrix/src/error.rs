//! 按原因区分的错误（AGENTS.md：失败即停，错误必须区分原因）。

use matrix_sdk::{HttpError, RefreshTokenError};
use matrix_sdk_ui::{encryption_sync_service, room_list_service, sync_service, timeline};

/// 本 crate 的结果类型。
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// Matrix 操作失败的原因。
///
/// 只保留可读的描述，便于跨线程传递和放进 ViewModel；原始错误在转换时已写进描述。
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// 连不上服务器：网络断开、隧道未就绪或请求超时。
    #[error("连不上 Matrix 服务器：{0}")]
    Transport(String),

    /// 服务器拒绝了身份或权限：JWT 无效或过期、设备已被吊销、账号已停用、无权操作。
    #[error("Matrix 服务器拒绝了请求（{errcode}）：{message}")]
    Unauthorized {
        /// Matrix 错误码，如 `M_FORBIDDEN`、`M_UNKNOWN_TOKEN`、`M_USER_DEACTIVATED`；
        /// 服务器没给错误码时为空。
        errcode: String,
        /// 服务器给出的说明。
        message: String,
    },

    /// 前置条件不满足：服务器缺少必需的能力，或房间还没同步到本地。
    #[error("Matrix 尚未就绪：{0}")]
    NotReady(String),

    /// 服务器或 SDK 出错。
    #[error("Matrix 服务出错：{0}")]
    Upstream(String),

    /// 输入非法。
    #[error("输入非法：{0}")]
    InvalidInput(String),
}

impl From<&HttpError> for Error {
    fn from(error: &HttpError) -> Self {
        match error {
            HttpError::Reqwest(source) => Self::Transport(source.to_string()),
            HttpError::Cached(inner) => inner.as_ref().into(),
            HttpError::RefreshToken(RefreshTokenError::MatrixAuth(inner)) => inner.as_ref().into(),
            // 访问令牌已被拒绝，又没法刷新（会话没有刷新令牌等）。
            HttpError::RefreshToken(other) => Self::Unauthorized {
                errcode: String::new(),
                message: other.to_string(),
            },
            _ => match error.as_client_api_error() {
                Some(api) => match api.status_code.as_u16() {
                    401 | 403 => Self::Unauthorized {
                        errcode: api
                            .error_kind()
                            .map_or_else(String::new, |kind| kind.errcode().to_string()),
                        message: api.to_string(),
                    },
                    400 => Self::InvalidInput(api.to_string()),
                    _ => Self::Upstream(api.to_string()),
                },
                None => Self::Upstream(error.to_string()),
            },
        }
    }
}

impl From<HttpError> for Error {
    fn from(error: HttpError) -> Self {
        (&error).into()
    }
}

impl From<&matrix_sdk::Error> for Error {
    fn from(error: &matrix_sdk::Error) -> Self {
        match error {
            matrix_sdk::Error::Http(http) => http.as_ref().into(),
            matrix_sdk::Error::Identifier(source) => Self::InvalidInput(source.to_string()),
            other => Self::Upstream(other.to_string()),
        }
    }
}

impl From<matrix_sdk::Error> for Error {
    fn from(error: matrix_sdk::Error) -> Self {
        (&error).into()
    }
}

impl From<&sync_service::Error> for Error {
    fn from(error: &sync_service::Error) -> Self {
        match error {
            sync_service::Error::RoomList(room_list_service::Error::SlidingSync(source))
            | sync_service::Error::EncryptionSync(encryption_sync_service::Error::SlidingSync(
                source,
            )) => source.into(),
            other => Self::Upstream(other.to_string()),
        }
    }
}

impl From<timeline::Error> for Error {
    fn from(error: timeline::Error) -> Self {
        match error {
            timeline::Error::InvalidAttachmentData | timeline::Error::InvalidAttachmentFileName => {
                Self::InvalidInput(error.to_string())
            }
            timeline::Error::Sdk(source) => (&source).into(),
            other => Self::Upstream(other.to_string()),
        }
    }
}
