//! App 登录和使用 Matrix（ADR-0009 的“接口：App 登录和使用 Matrix”）。只给经隧道接入的已配对设备：
//!
//! | 方法 | 地址 | 说明 |
//! | --- | --- | --- |
//! | POST | `<base>/_wonton/matrix/login-token` | 签一个 Tuwunel 登录用的一次性 JWT，连同 homeserver 地址 |
//! | 任意 | `matrix.<base>/_matrix/client/…`、`/_matrix/media/…` | Matrix 客户端接口，转发给 Tuwunel |
//!
//! 登录令牌能换出主人的 Matrix 账号，所以接口只认隧道身份（局域网浏览器的核心会话不行），用 `POST` 让核心
//! 域名上的跨站修改拦截对它生效，也不发 CORS 头，插件网页读不到响应。核心没有接 Tuwunel 时两个入口都如实报
//! `matrix_unavailable`。

use std::sync::Arc;

use axum::extract::{Request, State};
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use hyper::StatusCode;
use hyper::header::HeaderValue;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::api::error_response;
use super::{Gateway, TunnelPeer, client_addr, pages, proxy, pushers};
use crate::matrix::{JWT_LOGIN_TYPE, LOGIN_TOKEN_TTL_SECS};
use crate::plugin::Permission;

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
    let Some(matrix) = &gateway.matrix else {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            KIND_UNAVAILABLE,
            UNAVAILABLE,
        );
    };
    // M1 的已配对设备都属于主人。
    let localpart = gateway.owner.username();
    Json(LoginToken {
        homeserver_url: gateway.public.origin(&gateway.public.matrix_host()),
        server_name: matrix.server_name().to_owned(),
        user_id: matrix.user_id(localpart),
        login_type: JWT_LOGIN_TYPE.to_owned(),
        token: matrix.login_token(localpart),
        expires_in: LOGIN_TOKEN_TTL_SECS,
    })
    .into_response()
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
    forward_client_api(gateway, req).await
}

/// `matrix.wonton.internal`：插件网络里的 Bot（`WONTON_MATRIX_HOMESERVER`）。只转发声明了 `matrix.bot` 的
/// 插件的请求，按来源地址认出插件（见 [`crate::plugin::sources`]）；其余规则同 `matrix.<base>`。
pub async fn bot_request(gateway: &Gateway, req: Request) -> Response {
    let source = client_addr(&req).and_then(|addr| gateway.plugins.source(addr.ip()));
    match source {
        Some(source) if source.permissions.contains(&Permission::MatrixBot) => {
            // Bot 只能注册指向推送代理的 pusher，见 `pushers`。
            let req = if pushers::is_pushers_set(req.method(), req.uri().path()) {
                match pushers::check(req).await {
                    Ok(req) => req,
                    Err(refused) => return *refused,
                }
            } else {
                req
            };
            forward_client_api(gateway, req).await
        }
        Some(source) => matrix_error(
            StatusCode::FORBIDDEN,
            "M_FORBIDDEN",
            "permission_denied",
            &format!("插件 {} 没有申请 matrix.bot 权限", source.id),
        ),
        None => matrix_error(
            StatusCode::FORBIDDEN,
            "M_FORBIDDEN",
            "unauthenticated",
            "这个地址只给插件网络里的 Bot 用",
        ),
    }
}

/// 只转发客户端接口，先挡住可能被上游读成别的路径的写法。
async fn forward_client_api(gateway: &Gateway, req: Request) -> Response {
    let path = req.uri().path();
    if is_ambiguous(path) {
        return matrix_error(
            StatusCode::BAD_REQUEST,
            "M_UNRECOGNIZED",
            "invalid_input",
            "路径里不能有 . 或 .. 段、反斜杠，以及编码过的 .、/、\\",
        );
    }
    if !is_client_api(path) {
        return matrix_error(
            StatusCode::NOT_FOUND,
            "M_UNRECOGNIZED",
            "not_found",
            "这里只转发 Matrix 客户端接口（/_matrix/client/、/_matrix/media/）",
        );
    }
    let Some(matrix) = &gateway.matrix else {
        return matrix_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "M_UNKNOWN",
            KIND_UNAVAILABLE,
            UNAVAILABLE,
        );
    };
    let fwd = proxy::Forwarded {
        client: client_addr(&req),
        scheme: &gateway.public.scheme,
    };
    let endpoint = crate::plugin::upstreams::Endpoint::Tcp(matrix.authority().to_owned());
    let target = proxy::Target {
        endpoint: &endpoint,
        name: "Tuwunel",
    };
    match proxy::forward_or(&gateway.system_http, target, req, fwd).await {
        Ok(response) => response,
        Err(e) => matrix_forward_error(e),
    }
}

/// Matrix 接口本身不走响应头期限，但匹配要写全：期限如果放开，错误也按 Matrix 的 JSON 格式回。
fn matrix_forward_error(error: proxy::ForwardError) -> Response {
    match error {
        proxy::ForwardError::Unreachable(e) | proxy::ForwardError::SystemResolution(e) => {
            matrix_error(
                StatusCode::SERVICE_UNAVAILABLE,
                "M_UNKNOWN",
                KIND_UNAVAILABLE,
                &format!("Tuwunel 暂时无法访问：{e}"),
            )
        }
        proxy::ForwardError::HeaderTimeout => matrix_error(
            StatusCode::GATEWAY_TIMEOUT,
            "M_UNKNOWN",
            "upstream_timeout",
            "Tuwunel 迟迟没有回应，可以稍后重试",
        ),
    }
}

const KIND_UNAVAILABLE: &str = "matrix_unavailable";
const UNAVAILABLE: &str = "核心没有接入 Tuwunel";

/// 只转发客户端接口和媒体接口；联邦接口（`/_matrix/federation/`、`/_matrix/key/`）和管理接口
/// （`/_synapse/admin/`）不转。先用 [`is_ambiguous`] 挡住可能被上游读成别的路径的写法。
fn is_client_api(path: &str) -> bool {
    ["/_matrix/client/", "/_matrix/media/"]
        .iter()
        .any(|prefix| path.starts_with(prefix))
}

/// 上游可能把这些写法规范化成另一条路径，绕过前缀判断到达管理接口：按字面的 `/` 切段，百分号解码后是 `.` 或
/// `..` 的段（包括 `%2e%2e`、`.%2e`），以及反斜杠。实测 Tuwunel v1.9.4 都不规范化，这一层是纵深防御，不能误伤
/// 合法请求：编码过的 `/`、`\`（`%2F`、`%5C`）放行。ruma 把路径参数里的 `/`、`\` 编成它们，而 Megolm 会话 ID
/// 是标准 base64，大约一半带 `/`，按会话取密钥备份（`GET /_matrix/client/v3/room_keys/keys/{房间}/{会话}`）
/// 要用到。
fn is_ambiguous(path: &str) -> bool {
    path.contains('\\')
        || path
            .split('/')
            .any(|segment| matches!(percent_decode(segment).as_slice(), b"." | b".."))
}

/// 百分号解码一段路径：`%XX` 换成字节，不成对的 `%` 原样保留。
fn percent_decode(segment: &str) -> Vec<u8> {
    let hex = |b: u8| (b as char).to_digit(16).and_then(|d| u8::try_from(d).ok());
    let mut out = Vec::with_capacity(segment.len());
    let mut rest = segment.as_bytes();
    while let Some((&first, tail)) = rest.split_first() {
        if first == b'%'
            && let [high, low, after @ ..] = tail
            && let (Some(high), Some(low)) = (hex(*high), hex(*low))
        {
            out.push(high << 4 | low);
            rest = after;
        } else {
            out.push(first);
            rest = tail;
        }
    }
    out
}

pub(super) fn matrix_error(
    status: StatusCode,
    errcode: &str,
    kind: &'static str,
    message: &str,
) -> Response {
    let mut response =
        (status, Json(json!({"errcode": errcode, "error": message}))).into_response();
    response
        .headers_mut()
        .insert(pages::ERROR_HEADER, HeaderValue::from_static(kind));
    response
}

#[cfg(test)]
mod tests {
    #[test]
    fn forward_failures_keep_their_own_status_and_kind() {
        for (error, status, kind) in [
            (
                proxy::ForwardError::Unreachable("连不上".into()),
                StatusCode::SERVICE_UNAVAILABLE,
                KIND_UNAVAILABLE,
            ),
            (
                proxy::ForwardError::HeaderTimeout,
                StatusCode::GATEWAY_TIMEOUT,
                "upstream_timeout",
            ),
        ] {
            let response = matrix_forward_error(error);
            assert_eq!(response.status(), status);
            assert_eq!(response.headers()[pages::ERROR_HEADER], kind);
        }
    }

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
    fn paths_that_upstream_could_read_differently_are_refused() {
        for path in [
            "/_matrix/client/../../_synapse/admin/v1/users",
            "/_matrix/client/./v3/sync",
            "/_matrix/client/%2e%2e/%2E%2E/_synapse/admin",
            "/_matrix/client/.%2e/_synapse/admin",
            "/_matrix/client/%2e/v3/sync",
            "/_matrix/client\\..\\_synapse",
        ] {
            assert!(is_ambiguous(path), "{path}");
        }
        for path in [
            "/_matrix/client/v3/rooms/!abc:wonton.localhost/send/m.room.message/1",
            "/_matrix/client/v3/profile/@a.b:wonton.localhost/displayname",
            "/_matrix/media/v3/download/wonton.localhost/abc",
            // Megolm 会话 ID 里的 `/` 由 ruma 编成 %2F：按会话取密钥备份必须放行。
            "/_matrix/client/v3/room_keys/keys/!r:x/ab%2Fcd%2fef",
            "/_matrix/client/v3/x%5Cy",
            "/_matrix/client/v3/a..b/.c./%252e%252e",
            "/_matrix/client/v3/50%/%zz",
        ] {
            assert!(!is_ambiguous(path), "{path}");
        }
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
