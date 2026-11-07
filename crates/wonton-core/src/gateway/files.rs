//! 文件接口（ADR-0020 的“接口”），都在核心地址上，只接受经隧道的已配对设备，还要带 `X-Wonton-Files: 1`：
//!
//! | 方法 | 路径 | 说明 |
//! | --- | --- | --- |
//! | GET | `/_wonton/files/{id}` | 文件信息 |
//! | GET、HEAD | `/_wonton/files/{id}/content` | 字节。支持 `Range`、`If-Range`，带强 `ETag` |
//! | GET | `/_wonton/files/{id}/thumbnail` | 长边 800 像素的 JPEG |
//! | POST | `/_wonton/files/{id}/grants` | 请求体 `{"room": …}`，返回 `{"bots": […]}`：把这个房间的 Bot 加进访问范围 |
//! | DELETE | `/_wonton/files/{id}/grants/{bot}` | 把这个 Bot 移出访问范围 |
//! | POST | `/_wonton/files/{id}/send-to-devices` | 把链接发到“Wonton”私聊 |
//! | DELETE | `/_wonton/files/{id}` | 删掉编号，设备上传的文件核对后连文件一起删 |
//!
//! 隧道身份随 WebView 的每个请求自动带上，跨站带自定义请求头却要先过预检，这里不回 CORS 头，插件网页因此碰不到这些接口。
//! 上传在 [`uploads`]。文件内容不可信：响应一律 `nosniff`、`CSP: sandbox`，只有媒体写真实类型。

mod uploads;

use std::sync::Arc;

use axum::Router;
use axum::body::{Body, Bytes};
use axum::extract::{Extension, Path, Request, State};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete as delete_route, get, post};
use futures_util::stream;
use hyper::header::{self, HeaderMap, HeaderName, HeaderValue};
use hyper::{Method, StatusCode};
use iroh::EndpointId;
use serde_json::json;
use tokio::io::AsyncReadExt;
use wonton_protocol::{
    FILE_ERROR_HEADER, FILES_HEADER, FileError, FileErrorBody, FileFailure, FileGrantRequest,
    FileId,
};

use super::api::languages;
use super::{Gateway, TunnelPeer};
use crate::files::content::{Selection, attachment, select, served_type};
use crate::files::record::{Accessor, FileRecord};
use crate::files::resource::RemoteRequest;
use crate::files::share_fs::Opened;
use crate::files::{Content, Failure, FileEvent, Files, device_label};
use crate::l10n;
use crate::plugin::languages::Languages;

pub(super) use uploads::hook_request;

/// 流式发文件时每次读这么多。
const CHUNK_BYTES: usize = 64 << 10;
const NO_SNIFF: (HeaderName, HeaderValue) = (
    header::X_CONTENT_TYPE_OPTIONS,
    HeaderValue::from_static("nosniff"),
);
const SANDBOX: (HeaderName, HeaderValue) = (
    header::CONTENT_SECURITY_POLICY,
    HeaderValue::from_static("sandbox"),
);

/// 通过了 [`require_device`] 的设备和文件服务。
#[derive(Clone)]
struct Caller {
    device: EndpointId,
    files: Arc<Files>,
}

impl Caller {
    fn audit(&self, event: &'static str, file: &str, outcome: &Result<(), Failure>) {
        self.files.audit(&FileEvent {
            event,
            device: device_label(&self.device),
            file,
            range: None,
            bots: None,
            outcome: outcome_of(outcome),
        });
    }
}

fn outcome_of<T>(outcome: &Result<T, Failure>) -> &'static str {
    match outcome {
        Ok(_) => "ok",
        Err(failure) => failure.kind.as_str(),
    }
}

pub fn routes(gateway: Arc<Gateway>) -> Router<Arc<Gateway>> {
    Router::new()
        .route("/_wonton/files/{id}", get(info).delete(delete))
        .route("/_wonton/files/{id}/content", get(content))
        .route("/_wonton/files/{id}/thumbnail", get(thumbnail))
        .route("/_wonton/files/{id}/grants", post(grant))
        .route("/_wonton/files/{id}/grants/{bot}", delete_route(revoke))
        .route("/_wonton/files/{id}/send-to-devices", post(send_to_devices))
        .merge(uploads::routes())
        .layer(middleware::from_fn_with_state(gateway, require_device))
        .layer(middleware::map_response(no_sniff))
}

async fn no_sniff(mut response: Response) -> Response {
    let (name, value) = NO_SNIFF;
    response.headers_mut().insert(name, value);
    response
}

/// 错误响应：`{"error": {"kind": …, "message": …}}` 和 `X-Wonton-Error`。
pub(super) fn failure(failure: &Failure) -> Response {
    let status = failure
        .kind
        .http_status()
        .and_then(|code| StatusCode::from_u16(code).ok())
        .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    let body = FileErrorBody {
        error: FileError {
            kind: failure.kind,
            message: failure.message.clone(),
        },
    };
    (
        status,
        [(FILE_ERROR_HEADER, failure.kind.as_str())],
        axum::Json(body),
    )
        .into_response()
}

fn respond(result: Result<Response, Failure>) -> Response {
    result.unwrap_or_else(|e| failure(&e))
}

/// 经隧道的已配对设备才能过，每个请求都查设备登记表，吊销立即生效。文件的访问范围还是只有主人
/// （ADR-0045 的按人范围是后续任务）：家人的设备如实报 `not_shared`。
async fn require_device(
    State(gateway): State<Arc<Gateway>>,
    mut req: Request,
    next: Next,
) -> Response {
    let asked = req
        .headers()
        .get(FILES_HEADER)
        .is_some_and(|value| value.as_bytes() == b"1");
    let peer = req.extensions().get::<TunnelPeer>().copied();
    let device = peer
        .filter(|TunnelPeer(id)| gateway.tunnel.devices.contains(id))
        .map(|TunnelPeer(id)| id);
    let (true, Some(device)) = (asked, device) else {
        let langs = languages(req.headers());
        return failure(&Failure::new(
            FileFailure::Unauthenticated,
            l10n::text(
                &langs,
                "files-device-only",
                &[("header", FILES_HEADER.into())],
            ),
        ));
    };
    let person = gateway.device_person(peer);
    if person.is_none_or(|person| !person.is_owner()) {
        return failure(&Failure::new(
            FileFailure::NotShared,
            "文件接口还没有开放给家人（访问范围按人是后续任务）",
        ));
    }
    let Some(files) = gateway.files.clone() else {
        let langs = languages(req.headers());
        return failure(&Failure::new(
            FileFailure::ServiceUnavailable,
            l10n::text(&langs, "files-service-disabled", &[]),
        ));
    };
    req.extensions_mut().insert(Caller { device, files });
    next.run(req).await
}

/// 编号写得不对，和没有这个编号是一回事。
fn parse_id(id: &str, langs: &Languages) -> Result<FileId, Failure> {
    id.parse().map_err(|_| {
        Failure::new(
            FileFailure::FileUnknown,
            l10n::text(langs, "files-not-found", &[]),
        )
    })
}

async fn info(
    Extension(caller): Extension<Caller>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    let langs = languages(&headers);
    let info = async {
        let id = parse_id(&id, &langs)?;
        caller.files.info(&id, &Accessor::Owner, &langs).await
    };
    respond(info.await.map(|info| axum::Json(info).into_response()))
}

async fn delete(
    Extension(caller): Extension<Caller>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    let langs = languages(&headers);
    let result = match parse_id(&id, &langs) {
        Ok(parsed) => {
            let device = device_label(&caller.device);
            caller.files.delete(&parsed, device, &langs).await
        }
        Err(e) => {
            let failed = Err(e);
            caller.audit("file_delete", &id, &failed);
            failed
        }
    };
    respond(result.map(|()| StatusCode::NO_CONTENT.into_response()))
}

async fn grant(
    Extension(caller): Extension<Caller>,
    Path(id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let langs = languages(&headers);
    let bots = async {
        let parsed = parse_id(&id, &langs)?;
        let request: FileGrantRequest = serde_json::from_slice(&body).map_err(|e| {
            Failure::new(
                FileFailure::InvalidInput,
                l10n::text(
                    &langs,
                    "files-invalid-room-body",
                    &[("error", e.to_string().into())],
                ),
            )
        })?;
        caller
            .files
            .grant_room(&parsed, &request.room, &langs)
            .await
    }
    .await;
    caller.files.audit(&FileEvent {
        event: "file_grant",
        device: device_label(&caller.device),
        file: &id,
        range: None,
        bots: bots.as_ref().ok().map(Vec::as_slice),
        outcome: outcome_of(&bots),
    });
    respond(bots.map(|bots| axum::Json(json!({"bots": bots})).into_response()))
}

async fn revoke(
    Extension(caller): Extension<Caller>,
    Path((id, bot)): Path<(String, String)>,
    headers: HeaderMap,
) -> Response {
    let langs = languages(&headers);
    let revoked = async {
        let parsed = parse_id(&id, &langs)?;
        caller.files.revoke_bot(&parsed, &bot, &langs).await
    }
    .await;
    let bots = [bot];
    caller.files.audit(&FileEvent {
        event: "file_revoke",
        device: device_label(&caller.device),
        file: &id,
        range: None,
        bots: Some(&bots),
        outcome: outcome_of(&revoked),
    });
    respond(revoked.map(|()| StatusCode::NO_CONTENT.into_response()))
}

async fn send_to_devices(
    Extension(caller): Extension<Caller>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    let langs = languages(&headers);
    let sent = async {
        let parsed = parse_id(&id, &langs)?;
        caller.files.send_to_devices(&parsed, &langs).await
    }
    .await;
    caller.audit("file_send_to_devices", &id, &sent);
    respond(sent.map(|()| StatusCode::NO_CONTENT.into_response()))
}

async fn thumbnail(
    Extension(caller): Extension<Caller>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    let langs = languages(&headers);
    let jpeg = async {
        let parsed = parse_id(&id, &langs)?;
        let record = caller.files.authorize(&parsed, &Accessor::Owner, &langs)?;
        caller.files.thumbnail(&record, &langs).await
    };
    respond(jpeg.await.map(|jpeg| {
        let mut response = jpeg.into_response();
        let headers = response.headers_mut();
        headers.insert(header::CONTENT_TYPE, HeaderValue::from_static("image/jpeg"));
        let (name, value) = SANDBOX;
        headers.insert(name, value);
        response
    }))
}

/// 类型和附件的响应头。
fn presentation(headers: &mut HeaderMap, record: &FileRecord) {
    let (name, value) = SANDBOX;
    headers.insert(name, value);
    match served_type(&record.name, record.mime_type.as_deref()) {
        Some(mime) => {
            headers.insert(header::CONTENT_TYPE, HeaderValue::from_static(mime));
        }
        None => {
            headers.insert(
                header::CONTENT_TYPE,
                HeaderValue::from_static("application/octet-stream"),
            );
            if let Ok(value) = HeaderValue::from_str(&attachment(&record.name)) {
                headers.insert(header::CONTENT_DISPOSITION, value);
            }
        }
    }
}

/// 转给插件的 `Range`：只转一段。多段时插件回 `multipart/byteranges`，响应头却写着文件的类型，客户端收到的内容就坏了；去掉
/// 就是从头下，这是 HTTP 允许的。
fn single_range(headers: &HeaderMap) -> Option<HeaderValue> {
    let range = headers.get(header::RANGE)?;
    let spec = range.to_str().ok()?.strip_prefix("bytes=")?;
    (!spec.contains(',')).then(|| range.clone())
}

/// 审计里这次下载的结果。416 没有传字节，不算下载成功。
fn download_outcome(served: &Result<(Response, Option<String>), Failure>) -> &'static str {
    match served {
        Ok((response, _)) if response.status() == StatusCode::RANGE_NOT_SATISFIABLE => {
            "range_not_satisfiable"
        }
        other => outcome_of(other),
    }
}

/// `GET`、`HEAD /_wonton/files/{id}/content`。下载写审计，`HEAD` 不写：它没有传字节。
async fn content(
    Extension(caller): Extension<Caller>,
    Path(id): Path<String>,
    method: Method,
    headers: HeaderMap,
) -> Response {
    let langs = languages(&headers);
    let range = single_range(&headers);
    let if_range = range.as_ref().and(headers.get(header::IF_RANGE)).cloned();
    let request = RemoteRequest {
        method: method.clone(),
        range,
        if_range,
    };
    let served = async {
        let parsed = parse_id(&id, &langs)?;
        let record = caller.files.authorize(&parsed, &Accessor::Owner, &langs)?;
        let content = caller.files.content(&record, request, &langs).await?;
        let (mut response, range) = match content {
            Content::File(opened) => local_file(opened, &method, &headers, &langs).await,
            Content::Remote(upstream) => remote(upstream, &method),
            Content::Bytes(bytes) => in_memory(bytes, &method),
            Content::Unread { size } => (unread(size), None),
        };
        if response.status() == StatusCode::RANGE_NOT_SATISFIABLE {
            // 没有内容，用不着说内容是什么。
            let (name, value) = SANDBOX;
            response.headers_mut().insert(name, value);
        } else {
            presentation(response.headers_mut(), &record);
        }
        Ok::<_, Failure>((response, range))
    }
    .await;
    if method == Method::GET {
        caller.files.audit(&FileEvent {
            event: "file_download",
            device: device_label(&caller.device),
            file: &id,
            range: served.as_ref().ok().and_then(|(_, range)| range.clone()),
            bots: None,
            outcome: download_outcome(&served),
        });
    }
    respond(served.map(|(response, _)| response))
}

/// 本地文件：按 `Range` 选一段流式发出。返回响应和发的字节范围。
async fn local_file(
    opened: Opened,
    method: &Method,
    request: &HeaderMap,
    langs: &Languages,
) -> (Response, Option<String>) {
    let Opened { file, snapshot } = opened;
    let etag = snapshot.etag();
    let size = snapshot.size;
    let text = |name: HeaderName| request.get(name).and_then(|value| value.to_str().ok());
    let selection = select(text(header::RANGE), text(header::IF_RANGE), &etag, size);
    let (status, start, length, content_range) = match selection {
        Selection::Full => (StatusCode::OK, 0, size, None),
        Selection::Part { start, end } => (
            StatusCode::PARTIAL_CONTENT,
            start,
            end - start + 1,
            Some(format!("bytes {start}-{end}/{size}")),
        ),
        Selection::Unsatisfiable => {
            let response = (
                StatusCode::RANGE_NOT_SATISFIABLE,
                [(header::CONTENT_RANGE, format!("bytes */{size}"))],
            )
                .into_response();
            return (response, None);
        }
    };
    let range = (length > 0).then(|| format!("{start}-{}", start + length - 1));
    let body = if *method == Method::HEAD {
        Body::empty()
    } else {
        file_stream(file, start, length, langs)
    };
    let mut response = Response::new(body);
    *response.status_mut() = status;
    let headers = response.headers_mut();
    headers.insert(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));
    headers.insert(header::CONTENT_LENGTH, HeaderValue::from(length));
    if let Ok(value) = HeaderValue::from_str(&etag) {
        headers.insert(header::ETAG, value);
    }
    if let Some(value) = content_range.and_then(|text| HeaderValue::from_str(&text).ok()) {
        headers.insert(header::CONTENT_RANGE, value);
    }
    (response, range)
}

/// 流式读文件的一段。读出错或者文件中途变短时响应体提前结束，客户端按 `Content-Length` 看得出没传完，用 `Range` 接着下。
fn file_stream(file: std::fs::File, start: u64, length: u64, langs: &Languages) -> Body {
    let shortened = l10n::text(langs, "files-stream-shortened", &[]);
    let state = (file, start, length, None::<tokio::fs::File>);
    Body::from_stream(stream::try_unfold(
        state,
        move |(std_file, start, left, opened)| {
            let shortened = shortened.clone();
            async move {
                let mut file = match opened {
                    Some(file) => file,
                    None => {
                        let mut file = tokio::fs::File::from_std(std_file.try_clone()?);
                        tokio::io::AsyncSeekExt::seek(&mut file, std::io::SeekFrom::Start(start))
                            .await?;
                        file
                    }
                };
                if left == 0 {
                    return Ok::<_, std::io::Error>(None);
                }
                let want = usize::try_from(left)
                    .unwrap_or(CHUNK_BYTES)
                    .min(CHUNK_BYTES);
                let mut chunk = vec![0u8; want];
                let read = file.read(&mut chunk).await?;
                if read == 0 {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::UnexpectedEof,
                        shortened,
                    ));
                }
                chunk.truncate(read);
                Ok(Some((
                    Bytes::from(chunk),
                    (std_file, start, left - read as u64, Some(file)),
                )))
            }
        },
    ))
}

/// 插件内网地址的响应：只留说明这段字节的响应头。`Accept-Ranges` 只在插件支持时才有；弱 `ETag` 不转，`If-Range` 用不了它。
fn remote(upstream: hyper::Response<Body>, method: &Method) -> (Response, Option<String>) {
    let (parts, body) = upstream.into_parts();
    let content_range = parts
        .headers
        .get(header::CONTENT_RANGE)
        .filter(|_| parts.status == StatusCode::PARTIAL_CONTENT)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("bytes "))
        .and_then(|value| value.split('/').next())
        .map(str::to_owned);
    let body = if *method == Method::HEAD {
        Body::empty()
    } else {
        body
    };
    let mut response = Response::new(body);
    *response.status_mut() = parts.status;
    let headers = response.headers_mut();
    for name in [header::CONTENT_LENGTH, header::CONTENT_RANGE] {
        if let Some(value) = parts.headers.get(&name) {
            headers.insert(name, value.clone());
        }
    }
    if parts
        .headers
        .get(header::ACCEPT_RANGES)
        .is_some_and(|value| value.as_bytes() == b"bytes")
    {
        headers.insert(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));
    }
    if let Some(etag) = parts
        .headers
        .get(header::ETAG)
        .filter(|value| !value.as_bytes().starts_with(b"W/"))
    {
        headers.insert(header::ETAG, etag.clone());
    }
    (response, content_range)
}

/// `HEAD` 问插件资源：不读，大小是插件声明的。
fn unread(size: u64) -> Response {
    let mut response = Response::new(Body::empty());
    response
        .headers_mut()
        .insert(header::CONTENT_LENGTH, HeaderValue::from(size));
    response
}

/// `resources/read` 读到的内容：不支持 `Range`，从头下。
fn in_memory(bytes: Vec<u8>, method: &Method) -> (Response, Option<String>) {
    let length = bytes.len();
    let body = if *method == Method::HEAD {
        Body::empty()
    } else {
        Body::from(bytes)
    };
    let mut response = Response::new(body);
    response
        .headers_mut()
        .insert(header::CONTENT_LENGTH, HeaderValue::from(length));
    (response, None)
}
