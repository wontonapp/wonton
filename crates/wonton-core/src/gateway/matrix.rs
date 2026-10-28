//! App 登录和使用 Matrix（ADR-0009 的“接口：App 登录和使用 Matrix”）。只给经隧道接入的已配对设备：
//!
//! | 方法 | 地址 | 说明 |
//! | --- | --- | --- |
//! | POST | `<base>/_wonton/matrix/login-token` | 签一个 Tuwunel 登录用的一次性 JWT，连同 homeserver 地址 |
//! | 任意 | `matrix.<base>/_matrix/client/…`、`/_matrix/media/…` | Matrix 客户端接口，转发给 Tuwunel |
//!
//! 登录令牌能换出主人的 Matrix 账号，所以接口只认隧道身份（局域网浏览器的核心会话不行），用 `POST` 让核心
//! 域名上的跨站修改拦截对它生效，也不发 CORS 头，插件网页读不到响应。核心还没接入 Tuwunel（实现计划第 8
//! 项）时两个入口都如实报 `matrix_unavailable`。

use std::sync::Arc;

use axum::extract::{Request, State};
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use hyper::StatusCode;
use hyper::header::HeaderValue;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::api::error_response;
use super::{Gateway, TunnelPeer, pages};

/// 登录令牌接口的响应。客户端在 `expires_in` 秒内用它调一次 `POST {homeserver_url}/_matrix/client/v3/login`
/// （`{"type": login_type, "token": token}`）；设备自己登录，设备 ID 和刷新令牌都归设备。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoginToken {
    /// Matrix 客户端接口的地址：`<scheme>://matrix.<base>[:端口]`，经隧道访问，`Host` 原样保留。
    pub homeserver_url: String,
    /// Tuwunel 的 `server_name`，即 Matrix 用户 ID 冒号后的部分。
    pub server_name: String,
    /// 设备所属用户的 Matrix 用户 ID，如 `@owner:wonton.localhost`。M1 的已配对设备都属于主人。
    pub user_id: String,
    /// 登录请求的 `type`：`org.matrix.login.jwt`。
    pub login_type: String,
    /// 一次性的登录 JWT（Ed25519 签名，声明见 ADR-0009 的验证结果 M0-2）。
    pub token: String,
    /// 令牌还能用多少秒。
    pub expires_in: u64,
}

/// `POST /_wonton/matrix/login-token`。
pub async fn login_token(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
) -> Response {
    if gateway
        .device_user(peer.map(|Extension(peer)| peer))
        .is_none()
    {
        return error_response(
            StatusCode::UNAUTHORIZED,
            "unauthenticated",
            "只给经隧道接入的已配对设备签发 Matrix 登录令牌",
        );
    }
    error_response(
        StatusCode::SERVICE_UNAVAILABLE,
        KIND_UNAVAILABLE,
        UNAVAILABLE,
    )
}

/// `matrix.<base>`：只有已配对设备、只有客户端接口。错误按 Matrix 的格式给出，客户端的 SDK 能照常处理；
/// 不在这里用 `401`，免得 SDK 当成自己的访问令牌失效。
pub async fn client_request(gateway: &Gateway, req: Request) -> Response {
    if gateway
        .device_user(req.extensions().get().copied())
        .is_none()
    {
        return matrix_error(
            StatusCode::FORBIDDEN,
            "M_FORBIDDEN",
            "unauthenticated",
            "Matrix 只对经隧道接入的已配对设备开放",
        );
    }
    if !is_client_api(req.uri().path()) {
        return matrix_error(
            StatusCode::NOT_FOUND,
            "M_UNRECOGNIZED",
            "not_found",
            "这里只转发 Matrix 客户端接口（/_matrix/client/、/_matrix/media/）",
        );
    }
    matrix_error(
        StatusCode::SERVICE_UNAVAILABLE,
        "M_UNKNOWN",
        KIND_UNAVAILABLE,
        UNAVAILABLE,
    )
}

const KIND_UNAVAILABLE: &str = "matrix_unavailable";
const UNAVAILABLE: &str = "核心还没有接入 Tuwunel";

/// 只转发客户端接口和媒体接口；联邦接口（`/_matrix/federation/`、`/_matrix/key/`）和管理接口
/// （`/_synapse/admin/`）不转。
fn is_client_api(path: &str) -> bool {
    ["/_matrix/client/", "/_matrix/media/"]
        .iter()
        .any(|prefix| path.starts_with(prefix))
}

fn matrix_error(status: StatusCode, errcode: &str, kind: &'static str, message: &str) -> Response {
    let mut response =
        (status, Json(json!({"errcode": errcode, "error": message}))).into_response();
    response
        .headers_mut()
        .insert(pages::ERROR_HEADER, HeaderValue::from_static(kind));
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 契约：App 那一路按 ADR-0009 写的字段名解析响应。
    #[test]
    fn login_token_fields_match_the_documented_contract() {
        let token = LoginToken {
            homeserver_url: "http://matrix.wonton.localhost:18480".into(),
            server_name: "wonton.localhost".into(),
            user_id: "@owner:wonton.localhost".into(),
            login_type: "org.matrix.login.jwt".into(),
            token: "eyJ…".into(),
            expires_in: 120,
        };
        let value = serde_json::to_value(&token).unwrap();
        let mut keys: Vec<&str> = value
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            [
                "expires_in",
                "homeserver_url",
                "login_type",
                "server_name",
                "token",
                "user_id"
            ]
        );
        assert_eq!(serde_json::from_value::<LoginToken>(value).unwrap(), token);
    }

    #[test]
    fn only_client_and_media_apis_are_forwarded() {
        for path in [
            "/_matrix/client/versions",
            "/_matrix/client/v3/login",
            "/_matrix/media/v3/download/x/y",
        ] {
            assert!(is_client_api(path), "{path}");
        }
        for path in [
            "/_matrix/federation/v1/version",
            "/_matrix/key/v2/server",
            "/_synapse/admin/v1/register",
            "/_matrix/clientx",
            "/",
        ] {
            assert!(!is_client_api(path), "{path}");
        }
    }
}
