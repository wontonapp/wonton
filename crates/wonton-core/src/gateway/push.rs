//! 推送（ADR-0003 的“接口：推送”）：
//!
//! | 方法 | 地址 | 说明 |
//! | --- | --- | --- |
//! | PUT | `<base>/_wonton/push/registration` | 已配对设备登记推送订阅，拿到 pushkey 和推送代理的地址 |
//! | DELETE | `<base>/_wonton/push/registration` | 取消登记 |
//! | POST | `push.wonton.internal/_matrix/push/v1/notify` | 推送代理（Matrix Push Gateway API），只给 Tuwunel |
//!
//! 登记只认隧道身份，同 Matrix 登录令牌：局域网浏览器的核心会话不行，插件网页借隧道身份发的跨站请求被核心域名的
//! 跨站修改拦截挡住。Matrix 推送网关本身没有鉴权，凭据是 pushkey（核心生成的 256 位随机数）；推送代理另外只接受
//! Matrix 服务器的地址来的请求，局域网、插件网络和隧道来的都拒绝。

use std::net::SocketAddr;
use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{Request, State};
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json, Router};
use hyper::{Method, StatusCode};
use iroh::EndpointId;
use serde::Deserialize;
use serde_json::json;

use super::api::error_response;
use super::matrix::matrix_error;
use super::{Gateway, TunnelPeer, client_addr, from_service, request_host};
use crate::config::{HostKind, PUSH_GATEWAY_HOST};
use crate::push::{NotifyRequest, NotifyResponse, PushError, Subscription};

/// Matrix Push Gateway API 的通知接口。
const NOTIFY_PATH: &str = "/_matrix/push/v1/notify";

/// 通知请求体的上限：Tuwunel 的完整格式里带着消息正文，正常远小于它。
const MAX_NOTIFY_BYTES: usize = 64 * 1024;

const KIND_UNAVAILABLE: &str = "push_unavailable";
const UNAVAILABLE: &str = "核心没有启用推送代理";

/// 请求体：W3C Push API 的 `PushSubscription` JSON。
#[derive(Deserialize)]
struct SubscriptionBody {
    endpoint: String,
    keys: Keys,
}

#[derive(Deserialize)]
struct Keys {
    p256dh: String,
    auth: String,
}

/// 推送代理的地址：设备注册 pusher 时用它，网关只放行指向它的 pusher（`pushers`）。
pub(super) fn gateway_url() -> String {
    format!("http://{PUSH_GATEWAY_HOST}{NOTIFY_PATH}")
}

/// 经隧道的已配对设备。吊销后立即不再认它，哪怕请求还在一条没来得及断开的连接上。
fn paired_device(gateway: &Gateway, peer: Option<Extension<TunnelPeer>>) -> Option<EndpointId> {
    let Extension(TunnelPeer(id)) = peer?;
    gateway.tunnel.devices.contains(&id).then_some(id)
}

fn not_from_tunnel() -> Response {
    error_response(
        StatusCode::UNAUTHORIZED,
        "unauthenticated",
        "只有经隧道接入的已配对设备能登记推送",
    )
}

/// `PUT /_wonton/push/registration`。先鉴权再解析请求体。
pub async fn register(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    body: Bytes,
) -> Response {
    let Some(device) = paired_device(&gateway, peer) else {
        return not_from_tunnel();
    };
    let Some(push) = &gateway.push else {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            KIND_UNAVAILABLE,
            UNAVAILABLE,
        );
    };
    let subscription = serde_json::from_slice::<SubscriptionBody>(&body)
        .map_err(|e| PushError::Invalid(format!("请求体应为推送订阅的 JSON：{e}")))
        .and_then(|body| Subscription::new(&body.endpoint, &body.keys.p256dh, &body.keys.auth));
    let registered = subscription.and_then(|subscription| push.register(device, subscription));
    match registered {
        Ok(registration) => Json(json!({
            "pushkey": registration.pushkey,
            "gateway_url": gateway_url(),
        }))
        .into_response(),
        Err(e) => push_error(&e),
    }
}

/// `DELETE /_wonton/push/registration`。
pub async fn unregister(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
) -> Response {
    let Some(device) = paired_device(&gateway, peer) else {
        return not_from_tunnel();
    };
    let Some(push) = &gateway.push else {
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            KIND_UNAVAILABLE,
            UNAVAILABLE,
        );
    };
    match push.unregister(&device) {
        Ok(removed) => Json(json!({"removed": removed})).into_response(),
        Err(e) => push_error(&e),
    }
}

fn push_error(error: &PushError) -> Response {
    match error {
        PushError::Invalid(message) => {
            error_response(StatusCode::BAD_REQUEST, "invalid_input", message)
        }
        other => {
            tracing::error!(error = %other, "推送登记表写入失败");
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage",
                &other.to_string(),
            )
        }
    }
}

/// 吊销的设备不再收推送：删掉它的推送登记。推送代理每次推送前也会再查一次设备登记表。
pub(super) fn forget_device(gateway: &Gateway, device: &EndpointId) {
    if let Some(push) = &gateway.push
        && let Err(e) = push.unregister(device)
    {
        tracing::error!(device = %device.fmt_short(), error = %e, "删除已吊销设备的推送登记失败");
    }
}

/// TCP 来源是不是 Matrix 服务器（Tuwunel）。地址按核心配置里的 Matrix 服务器现查，Tuwunel 的容器重建后地址会变；
/// 核心没接 Matrix 时谁都不是。
async fn from_matrix_server(gateway: &Gateway, peer: Option<SocketAddr>) -> bool {
    let authority = gateway.matrix.as_ref().map(|matrix| matrix.authority());
    from_service(authority, peer, "Matrix 服务器").await
}

/// 专用监听只服务推送 Host，不暴露主网关的管理接口和反向代理。
pub fn router(gateway: Arc<Gateway>) -> Router {
    Router::new().fallback(move |req: Request| {
        let gateway = gateway.clone();
        async move {
            if gateway.public.classify(&request_host(&req)) != HostKind::Push {
                return StatusCode::NOT_FOUND.into_response();
            }
            gateway_request(&gateway, req).await
        }
    })
}

/// `push.wonton.internal`：推送代理。错误按 Matrix 的格式给出。
pub async fn gateway_request(gateway: &Gateway, req: Request) -> Response {
    if req.method() != Method::POST || req.uri().path() != NOTIFY_PATH {
        return matrix_error(
            StatusCode::NOT_FOUND,
            "M_UNRECOGNIZED",
            "not_found",
            "推送代理只有 POST /_matrix/push/v1/notify",
        );
    }
    // 先取出来：请求体不是 Sync，拿着 `&req` 等地址查询，处理请求的 future 就不是 Send。
    let tunnel = req.extensions().get::<TunnelPeer>().is_some();
    let peer = client_addr(&req);
    if tunnel || !from_matrix_server(gateway, peer).await {
        return matrix_error(
            StatusCode::FORBIDDEN,
            "M_FORBIDDEN",
            "unauthenticated",
            "推送代理只给 Matrix 服务器用",
        );
    }
    let Some(push) = &gateway.push else {
        return matrix_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "M_UNKNOWN",
            KIND_UNAVAILABLE,
            UNAVAILABLE,
        );
    };
    let request = match axum::body::to_bytes(req.into_body(), MAX_NOTIFY_BYTES).await {
        Ok(body) => serde_json::from_slice::<NotifyRequest>(&body).map_err(|e| e.to_string()),
        Err(e) => Err(format!("读请求体失败：{e}")),
    };
    let request = match request {
        Ok(request) => request,
        Err(message) => {
            return matrix_error(
                StatusCode::BAD_REQUEST,
                "M_BAD_JSON",
                "invalid_input",
                &format!("不是推送网关的通知请求：{message}"),
            );
        }
    };
    match push.notify(&request.notification).await {
        Ok(rejected) => Json(NotifyResponse { rejected }).into_response(),
        Err(e) => matrix_error(
            StatusCode::BAD_GATEWAY,
            "M_UNKNOWN",
            KIND_UNAVAILABLE,
            &e.to_string(),
        ),
    }
}
