//! 上传（ADR-0020 的“上传”）：设备经 tus 1.0 传字节，这里鉴权后转发给 tusd，tusd 经钩子问核心。
//!
//! | 方法 | 地址 | 说明 |
//! | --- | --- | --- |
//! | POST | `<base>/_wonton/files/uploads/` | 创建上传。核心发编号，放不下时 507 `storage_full` |
//! | HEAD、PATCH、DELETE | `<base>/_wonton/files/uploads/{id}` | 问进度、接着传、取消 |
//! | POST | `files-hooks.wonton.internal/` | tusd 的 HTTP 钩子 |
//!
//! tusd 对协议错误（比如偏移对不上）的响应原样转给客户端：那是 tus 协议的一部分。钩子没有鉴权头，凭据是 TCP 来源：
//! 同推送代理，只接受 tusd 的地址，隧道、局域网和插件网络来的都拒绝。

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::extract::{Extension, Path, Request, State};
use axum::response::{IntoResponse, Response};
use axum::routing::{head, post};
use http_body_util::BodyExt;
use hyper::header::{HeaderName, HeaderValue};
use hyper::{Method, StatusCode};
use serde::Deserialize;
use serde_json::json;
use wonton_protocol::{
    FILE_ERROR_HEADER, FILES_HEADER, FileError, FileErrorBody, FileFailure, FileId,
};

use super::super::api::languages;
use super::{Caller, failure, parse_id, respond};
use crate::files::uploads::{NewUpload, UploadState};
use crate::files::{Failure, Files, device_label, new_upload_id};
use crate::gateway::{Gateway, TunnelPeer, client_addr, from_service, proxy};
use crate::l10n;
use crate::plugin::languages::Languages;

const TUS_RESUMABLE: &str = "tus-resumable";
const UPLOAD_OFFSET: &str = "upload-offset";
const UPLOAD_LENGTH: &str = "upload-length";
/// 网关告诉 tusd（再经钩子告诉核心）这个上传的编号。客户端带的这个头会被去掉。
const UPLOAD_ID_HEADER: &str = "x-wonton-upload-id";
/// 网关告诉 tusd（再经钩子告诉核心）是哪台设备在传。客户端带的这个头会被去掉。
const DEVICE_HEADER: &str = "x-wonton-device";
/// tusd 认这个头，这里用不上，不让客户端改方法。
const METHOD_OVERRIDE: &str = "x-http-method-override";
/// 钩子请求体的上限：只有上传信息和请求头。
const MAX_HOOK_BYTES: usize = 1 << 20;
/// 问进度和取消最多等 tusd 这么久：取消占着这个上传的队，tusd 卡住不能把放文件也卡住。传字节的 PATCH 没有上限。
const TUSD_CONTROL_TIMEOUT: Duration = if cfg!(test) {
    Duration::from_secs(2)
} else {
    Duration::from_secs(30)
};

pub(super) fn routes() -> Router<Arc<Gateway>> {
    Router::new()
        .route("/_wonton/files/uploads/", post(create))
        .route(
            "/_wonton/files/uploads/{id}",
            head(resume).patch(resume).delete(resume),
        )
}

/// tus 客户端必带的头。客户端核心在 Rust 里发请求，WebView 里的跨站请求带不上它（ADR-0020 的“鉴权”）。
fn check_tus(req: &Request, langs: &Languages) -> Result<(), Failure> {
    let version = req.headers().get(TUS_RESUMABLE);
    if version.is_some_and(|value| value.as_bytes() == b"1.0.0") {
        return Ok(());
    }
    Err(Failure::new(
        FileFailure::InvalidInput,
        l10n::text(langs, "files-tusd-required", &[]),
    ))
}

/// 转发给 tusd。
async fn forward(
    gateway: &Gateway,
    caller: &Caller,
    mut req: Request,
    new: Option<&FileId>,
    langs: &Languages,
) -> Result<Response, Failure> {
    let authority = gateway.tusd.as_deref().ok_or_else(|| {
        Failure::new(
            FileFailure::ServiceUnavailable,
            l10n::text(langs, "files-tusd-unconfigured", &[]),
        )
    })?;
    let method = req.method().clone();
    let headers = req.headers_mut();
    headers.remove(FILES_HEADER);
    headers.remove(METHOD_OVERRIDE);
    headers.remove(UPLOAD_ID_HEADER);
    if let Some(new) = new
        && let Ok(new) = HeaderValue::from_str(new.as_str())
    {
        headers.insert(UPLOAD_ID_HEADER, new);
    }
    if let Ok(device) = HeaderValue::from_str(&device_label(&caller.device)) {
        headers.insert(DEVICE_HEADER, device);
    }
    let fwd = proxy::Forwarded {
        client: client_addr(&req),
        scheme: &gateway.public.scheme,
    };
    let endpoint = crate::plugin::upstreams::Endpoint::Tcp(authority.to_owned());
    let target = proxy::Target {
        endpoint: &endpoint,
        name: "tusd",
    };
    let unavailable = |why: String| Failure::new(FileFailure::ServiceUnavailable, why);
    // 转发失败可能是客户端自己断了，那不算 tusd 的问题。
    let broke = Arc::new(AtomicBool::new(false));
    let watching = broke.clone();
    let req = req.map(|body| {
        Body::new(body.map_err(move |e| {
            watching.store(true, Ordering::Relaxed);
            e
        }))
    });
    let forwarded = proxy::forward_or(&gateway.system_http, target, req, fwd, langs);
    let forwarded = if matches!(method, Method::HEAD | Method::DELETE) {
        tokio::time::timeout(TUSD_CONTROL_TIMEOUT, forwarded)
            .await
            .map_err(|_| unavailable(format!("tusd {TUSD_CONTROL_TIMEOUT:?} 内没有回答")))?
    } else {
        forwarded.await
    };
    forwarded.map_err(|e| tusd_failure(broke.load(Ordering::Relaxed), e, langs))
}

fn tusd_failure(broke: bool, error: proxy::ForwardError, langs: &Languages) -> Failure {
    if broke {
        return Failure::new(
            FileFailure::InvalidInput,
            format!("请求体没有传完，客户端可能断开了：{error}"),
        );
    }
    match error {
        // 沿用 service_unavailable：整套服务重启的几秒里客户端会暂停后接着传（`pausable`），
        // tusd 卡住超过期限时重试同样成立，不为它新加协议类别。
        proxy::ForwardError::Unreachable(e) => {
            Failure::new(FileFailure::ServiceUnavailable, format!("连不上 tusd：{e}"))
        }
        proxy::ForwardError::SystemResolution(e) => Failure::new(FileFailure::UpstreamError, e),
        proxy::ForwardError::HeaderTimeout => Failure::new(
            FileFailure::ServiceUnavailable,
            l10n::text(langs, "files-tusd-timeout", &[]),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tusd_failures_stay_in_their_own_kinds() {
        let langs = Languages::from_header(None);
        for (broke, error, kind) in [
            (
                false,
                proxy::ForwardError::Unreachable("连不上".into()),
                FileFailure::ServiceUnavailable,
            ),
            (
                false,
                proxy::ForwardError::HeaderTimeout,
                FileFailure::ServiceUnavailable,
            ),
            (
                true,
                proxy::ForwardError::HeaderTimeout,
                FileFailure::InvalidInput,
            ),
        ] {
            let failure = tusd_failure(broke, error, &langs);
            assert_eq!(failure.kind, kind);
        }
    }
}

/// 客户端还没拿到编号的上传。它接不上也取消不了，部分数据不留着占盘 24 小时、让重传报 `storage_full`。请求断了 handler 会被丢掉，所以
/// 没说要留下就在单独的任务里作废。
struct Unclaimed {
    files: Arc<Files>,
    id: FileId,
    settled: bool,
}

impl Unclaimed {
    fn new(files: &Arc<Files>, id: &FileId) -> Self {
        Self {
            files: files.clone(),
            id: id.clone(),
            settled: false,
        }
    }

    fn spawn_abandon(&mut self) -> tokio::task::JoinHandle<()> {
        self.settled = true;
        let (files, id) = (self.files.clone(), self.id.clone());
        tokio::spawn(async move {
            let turn = files.upload_turn(&id).await;
            files.upload_unclaimed(&turn).await;
        })
    }

    /// 客户端拿不到编号了，做完再回答。
    async fn abandon(mut self) {
        if let Err(e) = self.spawn_abandon().await {
            tracing::warn!(file = %self.id, error = %e, "作废上传的任务中止");
        }
    }

    fn keep(mut self) {
        self.settled = true;
    }
}

impl Drop for Unclaimed {
    fn drop(&mut self) {
        if !self.settled {
            drop(self.spawn_abandon());
        }
    }
}

async fn create(
    State(gateway): State<Arc<Gateway>>,
    Extension(caller): Extension<Caller>,
    req: Request,
) -> Response {
    let langs = languages(req.headers());
    respond(create_upload(&gateway, &caller, req, &langs).await)
}

async fn create_upload(
    gateway: &Gateway,
    caller: &Caller,
    req: Request,
    langs: &Languages,
) -> Result<Response, Failure> {
    check_tus(&req, langs)?;
    let id = new_upload_id();
    let unclaimed = Unclaimed::new(&caller.files, &id);
    let response = match forward(gateway, caller, req, Some(&id), langs).await {
        Ok(response) => response,
        Err(e) => {
            unclaimed.abandon().await;
            return Err(e);
        }
    };
    let created = response
        .headers()
        .get(hyper::header::LOCATION)
        .and_then(|location| location.to_str().ok())
        .and_then(|location| location.rsplit('/').next())
        .and_then(|id| id.parse::<FileId>().ok());
    // 创建时就带了字节，写的时候磁盘满了。按还差多少算，不是声明的长度：写了大半以后网络断了引起的 500 不是磁盘的事。
    let full = if response.status() == StatusCode::INTERNAL_SERVER_ERROR {
        caller.files.upload_out_of_space(&id, langs).await
    } else {
        None
    };
    if full.is_some() || created.is_none() {
        unclaimed.abandon().await;
    } else {
        unclaimed.keep();
    }
    if let Some(full) = full {
        return Err(full);
    }
    if response.status() == StatusCode::CREATED
        && let Some(id) = created
    {
        // 空文件，或者创建时就带上了全部字节：不会再有 PATCH。上传已经创建了，这里失败也照样回 tusd 的响应：客户端要靠
        // `Location` 问进度，问的时候会再放一次。
        let offset = header_u64(&response, UPLOAD_OFFSET).unwrap_or(0);
        if let Err(e) = finish_if_complete(caller, &id, offset, langs).await {
            tracing::error!(file = %id, error = %e, "放创建时就传完的文件失败，客户端问进度时再试");
        }
    }
    Ok(response)
}

async fn finish_if_complete(
    caller: &Caller,
    id: &FileId,
    offset: u64,
    langs: &Languages,
) -> Result<(), Failure> {
    if let UploadState::Pending { size, .. } = caller.files.upload_state(id).await?
        && offset == size
    {
        caller.files.finish_upload(id, langs).await?;
    }
    Ok(())
}

fn already_finished(langs: &Languages, id: &FileId) -> Failure {
    Failure::new(
        FileFailure::InvalidInput,
        l10n::text(
            langs,
            "files-upload-finished",
            &[("id", id.as_str().into())],
        ),
    )
}

fn header_u64(response: &Response, name: &str) -> Option<u64> {
    response.headers().get(name)?.to_str().ok()?.parse().ok()
}

/// 已经放好的上传再问进度：tusd 那边已经没有它了。
fn finished_response(size: u64) -> Response {
    let headers: [(HeaderName, HeaderValue); 4] = [
        (
            HeaderName::from_static(TUS_RESUMABLE),
            HeaderValue::from_static("1.0.0"),
        ),
        (
            HeaderName::from_static(UPLOAD_OFFSET),
            HeaderValue::from(size),
        ),
        (
            HeaderName::from_static(UPLOAD_LENGTH),
            HeaderValue::from(size),
        ),
        (
            hyper::header::CACHE_CONTROL,
            HeaderValue::from_static("no-store"),
        ),
    ];
    let mut response = StatusCode::OK.into_response();
    response.headers_mut().extend(headers);
    response
}

fn cancelled_response() -> Response {
    let mut response = StatusCode::NO_CONTENT.into_response();
    response.headers_mut().insert(
        HeaderName::from_static(TUS_RESUMABLE),
        HeaderValue::from_static("1.0.0"),
    );
    response
}

async fn resume(
    State(gateway): State<Arc<Gateway>>,
    Extension(caller): Extension<Caller>,
    Path(id): Path<String>,
    req: Request,
) -> Response {
    let langs = languages(req.headers());
    // 取消在单独的任务里：请求断了 handler 会被丢掉，这个上传的队也跟着放开，清理或者重试就会和还在跑的作废并发。
    if req.method() == Method::DELETE {
        let langs = langs.clone();
        let cancelling = tokio::spawn(async move {
            respond(resume_upload(&gateway, &caller, &id, req, &langs).await)
        });
        return cancelling.await.unwrap_or_else(|e| {
            failure(&Failure::new(
                FileFailure::Internal,
                format!("取消上传的任务中止：{e}"),
            ))
        });
    }
    respond(resume_upload(&gateway, &caller, &id, req, &langs).await)
}

async fn resume_upload(
    gateway: &Gateway,
    caller: &Caller,
    id: &str,
    req: Request,
    langs: &Languages,
) -> Result<Response, Failure> {
    check_tus(&req, langs)?;
    let id = parse_id(id, langs)?;
    let method = req.method().clone();
    // 取消全程占着这个上传：tusd 删了数据之后，放文件不能再接着放。
    let cancelling = if method == Method::DELETE {
        Some(caller.files.upload_turn(&id).await)
    } else {
        None
    };
    let size = match caller.files.upload_state(&id).await? {
        UploadState::Pending { size, device } => {
            // 没传完的上传只属于创建它的设备：编号被别的设备知道了，也接不上、取消不了。
            if device != device_label(&caller.device) {
                return Err(Failure::new(
                    FileFailure::NotShared,
                    l10n::text(langs, "files-upload-other-device", &[]),
                ));
            }
            // 判定很久没有进度靠这个，见 `Uploads::seen`。放在核对设备之后：别的设备不能续这个上传的命。
            if method != Method::DELETE && !caller.files.upload_seen(&id).await {
                return Err(Failure::new(
                    FileFailure::FileUnknown,
                    l10n::text(langs, "files-upload-stale", &[]),
                ));
            }
            size
        }
        UploadState::Finished { size } if method == Method::HEAD => {
            return Ok(finished_response(size));
        }
        UploadState::Finished { .. } => return Err(already_finished(langs, &id)),
        UploadState::Unknown => {
            return Err(Failure::new(
                FileFailure::FileUnknown,
                l10n::text(langs, "files-not-found", &[]),
            ));
        }
    };
    let response = forward(gateway, caller, req, None, langs).await?;
    let status = response.status();
    if status == StatusCode::NOT_FOUND {
        let waited;
        let turn = match &cancelling {
            Some(turn) => turn,
            None => {
                waited = caller.files.upload_turn(&id).await;
                &waited
            }
        };
        return match caller.files.settle_lost_upload(turn).await? {
            UploadState::Finished { size } if method == Method::HEAD => Ok(finished_response(size)),
            UploadState::Finished { .. } => Err(already_finished(langs, &id)),
            // 客户端要取消，这个上传已经取消掉了。
            _ if method == Method::DELETE => Ok(cancelled_response()),
            _ => Err(Failure::new(
                FileFailure::FileUnknown,
                l10n::text(langs, "files-tusd-gone", &[]),
            )),
        };
    }
    if method == Method::PATCH
        && status == StatusCode::INTERNAL_SERVER_ERROR
        && let Some(full) = caller.files.upload_out_of_space(&id, langs).await
    {
        return Err(full);
    }
    if let Some(turn) = &cancelling
        && status == StatusCode::NO_CONTENT
    {
        caller.files.upload_cancelled(turn).await;
    }
    // 最后一个 PATCH 回 204 就等于文件可用。上次放失败了，客户端问进度时再放一次，放不了就如实说。
    let answered = (method == Method::PATCH && status == StatusCode::NO_CONTENT)
        || (method == Method::HEAD && status == StatusCode::OK);
    if answered && header_u64(&response, UPLOAD_OFFSET) == Some(size) {
        caller.files.finish_upload(&id, langs).await?;
    }
    Ok(response)
}

/// tusd 钩子的请求体，只列用到的字段。
#[derive(Deserialize)]
struct Hook {
    #[serde(rename = "Type")]
    kind: String,
    #[serde(rename = "Event")]
    event: HookEvent,
}

#[derive(Deserialize)]
struct HookEvent {
    #[serde(rename = "Upload")]
    upload: HookUpload,
    #[serde(rename = "HTTPRequest")]
    request: HookRequest,
}

#[derive(Deserialize)]
struct HookUpload {
    /// post-finish 才有：pre-create 时还没有编号。
    #[serde(rename = "ID", default)]
    id: String,
    #[serde(rename = "Size", default)]
    size: u64,
    #[serde(rename = "SizeIsDeferred", default)]
    deferred: bool,
    #[serde(rename = "MetaData", default)]
    metadata: HashMap<String, String>,
}

#[derive(Deserialize)]
struct HookRequest {
    #[serde(rename = "Header", default)]
    header: HashMap<String, Vec<String>>,
}

impl HookRequest {
    /// tusd 用 Go 的规范写法，不靠它：请求头不分大小写。
    fn header(&self, name: &str) -> Option<&str> {
        let (_, values) = self
            .header
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))?;
        values.first().map(String::as_str)
    }

    fn device(&self) -> Option<&str> {
        self.header(DEVICE_HEADER)
    }

    fn upload_id(&self) -> Option<FileId> {
        self.header(UPLOAD_ID_HEADER)?.parse().ok()
    }

    /// 客户端请求的语言：网关把请求原样转给 tusd，钩子里的 `HTTPRequest` 带着原来的请求头。
    fn languages(&self) -> Languages {
        Languages::from_header(self.header(hyper::header::ACCEPT_LANGUAGE.as_str()))
    }
}

/// pre-create 的回答：tusd 把 `HTTPResponse` 原样转给客户端（实测，ADR-0020 的“验证结果”）。
fn hook_rejection(failed: &Failure) -> serde_json::Value {
    let body = FileErrorBody {
        error: FileError {
            kind: failed.kind,
            message: failed.message.clone(),
        },
    };
    json!({
        "RejectUpload": true,
        "HTTPResponse": {
            "StatusCode": failed.kind.http_status().unwrap_or(500),
            "Body": serde_json::to_string(&body).unwrap_or_default(),
            "Header": {
                "Content-Type": "application/json",
                "X-Content-Type-Options": "nosniff",
                FILE_ERROR_HEADER: failed.kind.as_str(),
            },
        },
    })
}

async fn pre_create(files: &Arc<Files>, event: &HookEvent) -> serde_json::Value {
    let upload = &event.upload;
    // 钩子本身没有 Accept-Language：tusd 把客户端的请求头原样带回来，按它挑语言。
    let langs = event.request.languages();
    let created = match (event.request.device(), event.request.upload_id()) {
        (Some(device), Some(id)) => {
            files
                .create_upload(
                    NewUpload {
                        id: &id,
                        name: upload.metadata.get("filename").map(String::as_str),
                        mime_type: upload.metadata.get("filetype").map(String::as_str),
                        size: (!upload.deferred).then_some(upload.size),
                        device,
                    },
                    &langs,
                )
                .await
        }
        _ => Err(Failure::new(
            FileFailure::Unauthenticated,
            l10n::text(&langs, "files-hook-not-from-gateway", &[]),
        )),
    };
    match created {
        Ok(id) => json!({"ChangeFileInfo": {"ID": id}}),
        Err(e) => hook_rejection(&e),
    }
}

/// `files-hooks.wonton.internal`：tusd 的钩子。
pub(in crate::gateway) async fn hook_request(gateway: &Gateway, req: Request) -> Response {
    // 钩子级别的拒绝不针对哪个上传，客户端看不到：按英文说。
    let langs = Languages::from_header(None);
    // 先取出来：请求体不是 Sync，拿着 `&req` 等地址查询，处理请求的 future 就不是 Send。
    let tunnel = req.extensions().get::<TunnelPeer>().is_some();
    let peer = client_addr(&req);
    let authority = gateway.tusd.as_deref();
    if tunnel || !from_service(authority, peer, "tusd").await {
        return failure(&Failure::new(
            FileFailure::PermissionDenied,
            l10n::text(&langs, "files-hook-tusd-only", &[]),
        ));
    }
    let Some(files) = &gateway.files else {
        return failure(&Failure::new(
            FileFailure::ServiceUnavailable,
            l10n::text(&langs, "files-service-disabled", &[]),
        ));
    };
    let body = axum::body::to_bytes(req.into_body(), MAX_HOOK_BYTES).await;
    let hook = body
        .map_err(|e| e.to_string())
        .and_then(|body| serde_json::from_slice::<Hook>(&body).map_err(|e| e.to_string()));
    let hook = match hook {
        Ok(hook) => hook,
        Err(e) => {
            return failure(&Failure::new(
                FileFailure::InvalidInput,
                format!("不是 tusd 的钩子请求：{e}"),
            ));
        }
    };
    match hook.kind.as_str() {
        "pre-create" => axum::Json(pre_create(files, &hook.event).await).into_response(),
        // 不等放完：客户端断了也要放好，网关在最后一个 PATCH 的响应之前也会等它。
        "post-finish" => {
            match hook.event.upload.id.parse::<FileId>() {
                Ok(id) => {
                    let files = files.clone();
                    tokio::spawn(async move {
                        if let Err(e) = files
                            .finish_upload(&id, &Languages::from_header(None))
                            .await
                        {
                            tracing::error!(file = %id, error = %e, "放传完的文件失败");
                        }
                    });
                }
                Err(_) => {
                    tracing::error!(id = hook.event.upload.id, "tusd 传完了一个不是核心发的编号")
                }
            }
            axum::Json(json!({})).into_response()
        }
        other => {
            tracing::warn!(hook = other, "没启用的 tusd 钩子");
            axum::Json(json!({})).into_response()
        }
    }
}
