//! 按原因区分的错误（AGENTS.md：失败即停，错误必须区分原因）。

use matrix_sdk::{
    HttpError, RefreshTokenError,
    ruma::api::error::{Error as ApiError, ErrorBody},
};
use matrix_sdk_ui::{
    encryption_sync_service, notification_client, room_list_service, sync_service, timeline,
};

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

    /// 前置条件不满足：服务器缺少必需的能力，房间还没同步到本地，或 Matrix 服务器暂时不可用（网关回 502–504）。
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
                Some(api) => api.into(),
                None => Self::Upstream(error.to_string()),
            },
        }
    }
}

impl From<&ApiError> for Error {
    fn from(api: &ApiError) -> Self {
        // Matrix 服务器的错误都是 JSON。不是 JSON 的回应来自路上：隧道拒绝这台设备时本地代理回 403，隧道断了回 502，
        // 这时请求没到 Matrix，不能说“Matrix 服务器拒绝了”。
        if matches!(api.body, ErrorBody::NotJson { .. }) {
            return Self::Transport(api.to_string());
        }
        match api.status_code.as_u16() {
            401 | 403 => Self::Unauthorized {
                errcode: api
                    .error_kind()
                    .map_or_else(String::new, |kind| kind.errcode().to_string()),
                message: api.to_string(),
            },
            400 => {
                tracing::warn!(%api, "服务器拒绝了请求");
                Self::InvalidInput("服务器不接受这个请求，检查填的内容后再试".to_owned())
            }
            // 核心的网关连不上 Tuwunel（重启、升级中）回 503，Tuwunel 迟迟不回回 504，都是 Matrix JSON：
            // 会自己好，归成未就绪，登录才会退避重试，不当成要用户处理的服务器错误。
            502..=504 => Self::NotReady(api.to_string()),
            _ => Self::Upstream(api.to_string()),
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
            matrix_sdk::Error::Identifier(source) => {
                tracing::warn!(%source, "ID 格式不对");
                Self::InvalidInput("ID 的格式不对，检查后再输一次".to_owned())
            }
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

impl From<notification_client::Error> for Error {
    fn from(error: notification_client::Error) -> Self {
        match error {
            notification_client::Error::SdkError(source) => (&source).into(),
            notification_client::Error::UnknownRoom => {
                Self::NotReady("推送说的房间还没同步到本机".to_owned())
            }
            other => Self::Upstream(other.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use matrix_sdk::ruma::api::EndpointError;

    use super::*;

    fn response(status: u16, body: &str) -> ApiError {
        let response = matrix_sdk::ruma::exports::http::Response::builder()
            .status(status)
            .body(body.as_bytes())
            .unwrap();
        ApiError::from_http_response(response)
    }

    #[test]
    fn answers_from_the_tunnel_are_not_blamed_on_the_matrix_server() {
        for status in [403, 502, 503] {
            let error = Error::from(&response(status, "服务端拒绝了本设备\n"));
            assert!(matches!(error, Error::Transport(_)), "{status}: {error:?}");
        }
    }

    #[test]
    fn answers_from_the_matrix_server_keep_their_meaning() {
        let body = r#"{"errcode":"M_FORBIDDEN","error":"no"}"#;
        assert_eq!(
            Error::from(&response(403, body)),
            Error::Unauthorized {
                errcode: "M_FORBIDDEN".to_owned(),
                message: "[403 / M_FORBIDDEN] no".to_owned(),
            }
        );
        assert!(matches!(
            Error::from(&response(500, r#"{"errcode":"M_UNKNOWN","error":"x"}"#)),
            Error::Upstream(_)
        ));
    }

    #[test]
    fn a_matrix_server_that_is_temporarily_unavailable_is_not_ready() {
        for status in [502, 503, 504] {
            let error = Error::from(&response(
                status,
                r#"{"errcode":"M_UNKNOWN","error":"Tuwunel 暂时无法访问"}"#,
            ));
            assert!(matches!(error, Error::NotReady(_)), "{status}: {error:?}");
        }
    }
}
