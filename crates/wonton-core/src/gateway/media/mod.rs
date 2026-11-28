//! 媒体接口 `/_wonton/api/media/*`（ADR-0113 第 2、3、4 节）：任务、播放会话、队列、条目、三条字节路由，
//! 和相册、影视原生浏览的转发（第 9、10 节），以及给壳挑的媒体源与共享对象。
//!
//! 只接经隧道的已配对设备，还要带 `X-Wonton-Device`，值是这台设备的隧道身份：带自定义请求头的跨站
//! 请求要先过预检，这里不回 CORS 头，插件网页因此碰不到这些接口（同文件接口）。人由核心按设备记录认，
//! 请求里写的 `person`、`device` 空着就取认出的，写了就必须相同。
//!
//! 错误体一律是 [`MediaFailure`] 信封；状态码只是提示，壳按信封里的类别出字。

mod browse;
mod bytes;
mod choices;
mod queue;
mod sessions;
mod streams;
mod tasks;

use std::sync::Arc;

use axum::Router;
use axum::extract::{FromRequestParts, Query, Request, State};
use axum::http::request::Parts;
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Json, Response};
use axum::routing::{get, post};
use hyper::StatusCode;
use serde::de::DeserializeOwned;
use wonton_protocol::media::api::MEDIA_API_PREFIX;
use wonton_protocol::media::{MediaFailure, MediaFailureCode, WONTON_DEVICE_HEADER};

use super::{Gateway, Person, TunnelPeer, matrix_user_id};
use crate::media::Media;
use crate::plugin::media_source::failure;

/// 通过了 [`require_device`] 的调用方。
#[derive(Clone)]
pub(super) struct Caller {
    /// 数据主体，同 `X-Wonton-User`（MXID）。
    person: String,
    device: String,
    who: Person,
    media: Arc<Media>,
}

impl Caller {
    /// 这个人能不能用某个插件（角色的可用范围）。
    fn allowed(&self, gateway: &Gateway) -> impl Fn(&str) -> bool + use<> {
        let apps = gateway.permissions(&self.who).open_apps;
        move |plugin| apps.allows_plugin(plugin)
    }

    /// 请求里写的人：空着取调用方，写了别人回 `permission_denied`。
    fn person(&self, asked: &str) -> Result<String, MediaFailure> {
        if asked.is_empty() || asked == self.person {
            return Ok(self.person.clone());
        }
        Err(failure(
            MediaFailureCode::PermissionDenied,
            format!("这台设备只能替 {} 操作", self.person),
        ))
    }

    fn device(&self, asked: &str) -> Result<String, MediaFailure> {
        if asked.is_empty() || asked == self.device {
            return Ok(self.device.clone());
        }
        Err(failure(
            MediaFailureCode::PermissionDenied,
            "只能替发请求的这台设备建任务、开播",
        ))
    }
}

pub(super) fn routes(gateway: Arc<Gateway>) -> Router<Arc<Gateway>> {
    let at = |path: &str| format!("{MEDIA_API_PREFIX}{path}");
    Router::new()
        .route(&at("/tasks"), post(tasks::create).get(tasks::list))
        .route(&at("/tasks/{id}"), get(tasks::read))
        .route(&at("/tasks/{id}/{action}"), post(tasks::act))
        .route(
            &at("/tasks/{id}/items/{item}/failure"),
            post(tasks::item_failure),
        )
        .route(
            &at("/tasks/{id}/uploads/{item}"),
            post(bytes::upload_start).patch(bytes::upload_write),
        )
        .route(&at("/tasks/{id}/items/{item}/content"), get(bytes::content))
        .route(&at("/sessions"), post(sessions::start).get(sessions::list))
        .route(&at("/sessions/{id}"), get(sessions::read))
        .route(&at("/sessions/{id}/control"), post(sessions::control))
        .route(&at("/sessions/{id}/progress"), post(sessions::progress))
        .route(&at("/sessions/{id}/stop"), post(sessions::stop))
        .route(&at("/sessions/{id}/stream"), get(streams::stream))
        .route(
            &at("/sessions/{id}/stream/{*rest}"),
            get(streams::stream_part),
        )
        .route(
            &at("/sessions/{id}/items/{item}/stream"),
            get(streams::queued_stream),
        )
        .route(
            &at("/sessions/{id}/items/{item}/stream/{*rest}"),
            get(streams::queued_stream_part),
        )
        .route(&at("/queue"), get(queue::read).post(queue::change))
        .route(&at("/sources"), get(choices::sources))
        .route(&at("/share-targets"), get(choices::share_targets))
        .route(&at("/items"), get(bytes::items))
        .route(&at("/items/{id}/artwork"), get(bytes::artwork))
        // 根路径（这家接哪些写操作）带不带结尾的 `/` 都认；`{*rest}` 不匹配空串。
        .route(&at("/library"), get(browse::library))
        .route(&at("/library/"), get(browse::library))
        .route(&at("/catalog"), get(browse::catalog))
        .route(&at("/catalog/"), get(browse::catalog))
        .route(
            &at("/library/{*rest}"),
            get(browse::library).post(browse::library),
        )
        .route(
            &at("/catalog/{*rest}"),
            get(browse::catalog).post(browse::catalog),
        )
        .layer(middleware::from_fn_with_state(gateway, require_device))
}

async fn require_device(
    State(gateway): State<Arc<Gateway>>,
    mut req: Request,
    next: Next,
) -> Response {
    let peer = req
        .extensions()
        .get::<TunnelPeer>()
        .copied()
        .filter(|TunnelPeer(id)| gateway.tunnel.devices.contains(id));
    let asked = req
        .headers()
        .get(WONTON_DEVICE_HEADER)
        .and_then(|value| value.to_str().ok());
    let (Some(peer), Some(asked)) = (peer, asked) else {
        return reply(&failure(
            MediaFailureCode::PermissionDenied,
            format!("媒体接口只给经隧道的已配对设备，还要带 {WONTON_DEVICE_HEADER}"),
        ));
    };
    let device = crate::files::device_label(&peer.0);
    if asked != device {
        return reply(&failure(
            MediaFailureCode::PermissionDenied,
            format!("{WONTON_DEVICE_HEADER} 和这台设备的隧道身份对不上"),
        ));
    }
    let Some(who) = gateway.device_person(Some(peer)) else {
        return reply(&failure(
            MediaFailureCode::PermissionDenied,
            "认不出这台设备是谁的",
        ));
    };
    let Some(media) = gateway.media.clone() else {
        return reply(&failure(
            MediaFailureCode::PluginUnavailable,
            "这台服务器没有开媒体服务",
        ));
    };
    let person = matrix_user_id(&gateway, &who.username);
    req.extensions_mut().insert(Caller {
        person,
        device,
        who,
        media,
    });
    next.run(req).await
}

/// 失败信封和一个提示用的状态码。
pub(super) fn reply(failure: &MediaFailure) -> Response {
    (status_of(&failure.code), Json(failure)).into_response()
}

/// 每个码都列出来、不留兜底：新加的码要在这里定状态码，不会悄悄落成 502 让壳以为核心坏了。
fn status_of(code: &MediaFailureCode) -> StatusCode {
    use MediaFailureCode as C;
    match code {
        C::PermissionDenied | C::LocalPermissionDenied => StatusCode::FORBIDDEN,
        C::MediaGone => StatusCode::NOT_FOUND,
        C::RateLimited => StatusCode::TOO_MANY_REQUESTS,
        C::PluginUnavailable | C::TransportUnavailable => StatusCode::SERVICE_UNAVAILABLE,
        C::ServerStorageFull | C::LocalStorageFull => StatusCode::INSUFFICIENT_STORAGE,
        C::UnsupportedByDevice => StatusCode::UNSUPPORTED_MEDIA_TYPE,
        C::Protocol => StatusCode::BAD_REQUEST,
        // 要用户选一家带 `source` 再来：请求缺东西，不是核心或上游坏了。
        C::SourceRequired => StatusCode::UNPROCESSABLE_ENTITY,
        // 核心自己回队列冲突时体是当前队列，不走信封；这个码是客户端核重放不了时给壳的。
        C::StaleRevision => StatusCode::CONFLICT,
        C::AuthenticationRequired
        | C::UpstreamError
        | C::LocalSourceUnavailable
        | C::LocalIo
        | C::SuspendedBySystem
        | C::Other(_) => StatusCode::BAD_GATEWAY,
    }
}

/// 查询串按约定的形状解析；解析不了回 `protocol` 信封，不用 axum 默认的纯文本 400。
pub(super) struct MediaQuery<T>(pub(super) T);

impl<T, S> FromRequestParts<S> for MediaQuery<T>
where
    T: DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        match Query::<T>::from_request_parts(parts, state).await {
            Ok(Query(value)) => Ok(Self(value)),
            Err(rejection) => Err(reply(&failure(
                MediaFailureCode::Protocol,
                format!("查询串不是约定的形状：{}", rejection.body_text()),
            ))),
        }
    }
}

/// 媒体库、状态机报的错折成信封：找不到的当 `media_gone`，不合法的转移当 `protocol`，其余是核心自己出错。
pub(super) fn service_failure(error: &dyn ServiceError) -> MediaFailure {
    failure(error.code(), error.to_string())
}

pub(super) trait ServiceError: std::fmt::Display {
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
