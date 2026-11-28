//! 上传、下载的字节路由（ADR-0113 第 2 节“字节路径”）和条目浏览：核心流式转给提供方，不落盘也不解码。
//!
//! - `POST /tasks/{id}/uploads/{item}`：无体。还没有上传号就用任务里的这一项补体向提供方开一个、记下号；
//!   有号先 `HEAD` 查断点，提供方不认（404）就重开。回当前 `upload_offset`。
//! - `PATCH /tasks/{id}/uploads/{item}`：按记下的号续写，体和 `Content-Range` 原样过；偏移对不上回 409 带
//!   当前偏移；收满就让提供方收尾，这一项记完成。
//! - `GET /tasks/{id}/items/{item}/content`：下载任务的字节，`Range`/`If-Range` 原样过。
//!
//! 播放的流在 [`super::streams`]。列出的条目由核心填 `source`；有封面的，`artwork_url` 换成核心的封面路由
//! （手机连不到提供方）。

use std::sync::Arc;

use axum::Extension;
use axum::body::Body;
use axum::extract::{Path, State};
use axum::response::{IntoResponse, Json, Response};
use hyper::header::{self, HeaderMap, HeaderName};
use hyper::{Method, StatusCode};
use serde::Deserialize;
use wonton_protocol::media::api;
use wonton_protocol::media::provider::{
    ArtworkKind, ArtworkQuery, ItemsQuery, item_artwork_path, item_download_path,
};
use wonton_protocol::media::uploads::{UPLOAD_OFFSET_HEADER, UploadOffset, UploadStart};
use wonton_protocol::media::{
    ItemState, MediaFailure, MediaFailureCode, MediaKind, MediaOperation, MediaTask, TaskKind,
    TaskTarget, UploadItem,
};

use super::tasks::{own_task, parse_task};
use super::{Caller, MediaQuery, reply, service_failure};
use crate::gateway::Gateway;
use crate::media::provider::ProviderRoute;
use crate::media::store::UploadBinding;
use crate::plugin::media_source::failure;

type Reply<T> = Result<T, MediaFailure>;

/// 往提供方带的请求头；其余（Cookie、隧道身份、客户端自己的凭据）一律不带。条件请求的两个头让客户端核的
/// 缩略图、海报缓存能拿到 304。
const PASS_REQUEST: [HeaderName; 8] = [
    header::RANGE,
    header::IF_RANGE,
    header::IF_NONE_MATCH,
    header::IF_MODIFIED_SINCE,
    header::CONTENT_RANGE,
    header::CONTENT_LENGTH,
    header::CONTENT_TYPE,
    header::ACCEPT,
];

pub(super) fn pass(headers: &HeaderMap) -> HeaderMap {
    let mut out = HeaderMap::new();
    for name in PASS_REQUEST {
        if let Some(value) = headers.get(&name) {
            out.insert(name, value.clone());
        }
    }
    out
}

fn offset(status: StatusCode, upload_offset: u64) -> Response {
    (status, Json(UploadOffset { upload_offset })).into_response()
}

fn upload_item(task: &MediaTask, item: &str) -> Reply<UploadItem> {
    let TaskTarget::Upload(items) = &task.target else {
        return Err(failure(MediaFailureCode::Protocol, "这不是上传任务"));
    };
    items
        .iter()
        .find(|entry| entry.item_id == item)
        .cloned()
        .ok_or_else(|| {
            failure(
                MediaFailureCode::MediaGone,
                format!("任务里没有 {item} 这一项"),
            )
        })
}

/// 上传任务记下的提供方；还没记就选一个能上传的记下。
async fn upload_route(
    gateway: &Gateway,
    caller: &Caller,
    task: &MediaTask,
) -> Reply<ProviderRoute> {
    let store = &caller.media.store;
    let known = store
        .task_provider(&task.person, task.id)
        .map_err(|e| service_failure(&e))?;
    let providers = &caller.media.providers;
    match known {
        Some(provider) => {
            providers
                .route_to(
                    &task.person,
                    caller.allowed(gateway),
                    &provider,
                    &MediaOperation::Upload,
                    None,
                )
                .await
        }
        None => {
            let route = providers
                .route(
                    &task.person,
                    caller.allowed(gateway),
                    &MediaOperation::Upload,
                    None,
                    None,
                )
                .await?;
            let bound = store
                .bind_task_provider(&task.person, task.id, &route.provider)
                .map_err(|e| service_failure(&e))?;
            if bound == route.provider {
                Ok(route)
            } else {
                providers
                    .route_to(
                        &task.person,
                        caller.allowed(gateway),
                        &bound,
                        &MediaOperation::Upload,
                        None,
                    )
                    .await
            }
        }
    }
}

/// 提供方那边的失败记到任务上（该退避的退避、不退避的直接失败），再原样回给设备。
fn fail_task(caller: &Caller, task: &MediaTask, failure: MediaFailure) -> Response {
    if let Err(e) = caller.media.tasks.fail(task.id, failure.clone()) {
        tracing::warn!(task = %task.id, error = %e, "提供方的失败没记到任务上");
    }
    reply(&failure)
}

pub(super) async fn upload_start(
    State(gateway): State<Arc<Gateway>>,
    Extension(caller): Extension<Caller>,
    Path((id, item)): Path<(String, String)>,
) -> Response {
    let task = match parse_task(&id).and_then(|task| own_task(&caller, task)) {
        Ok(task) => task,
        Err(failure) => return reply(&failure),
    };
    let entry = match upload_item(&task, &item) {
        Ok(entry) => entry,
        Err(failure) => return reply(&failure),
    };
    // 已经收尾的不再找提供方：它那边的上传号收尾后就没了，再查会被当成过期重开一份。
    if matches!(entry.state, ItemState::Done | ItemState::Skipped) {
        return offset(StatusCode::OK, entry.size);
    }
    // 先选好提供方再开始：配额按那家记，进 `Preparing` 时要知道是哪家。选不出来的失败等任务开始后再记，
    // 和传到一半出错一样走退避。
    let route = upload_route(&gateway, &caller, &task).await;
    let provider = route.as_ref().ok().map(|route| route.provider.as_str());
    if let Err(e) = caller.media.tasks.begin(task.id, provider) {
        return reply(&service_failure(&e));
    }
    let opened = match route {
        Ok(route) => open_upload(&caller, &route, &task, &entry).await,
        Err(failure) => Err(failure),
    };
    match opened {
        Ok(at) => offset(StatusCode::OK, at),
        Err(failure) => fail_task(&caller, &task, failure),
    }
}

/// 有号先查断点，没号或提供方不认就开新的；回当前偏移，并记到这一项上。已经收满（零字节的项、上次写完
/// 还没来得及收尾）就顺手收尾。
async fn open_upload(
    caller: &Caller,
    route: &ProviderRoute,
    task: &MediaTask,
    entry: &UploadItem,
) -> Reply<u64> {
    let store = &caller.media.store;
    let binding = store
        .upload_binding(&task.person, task.id, &entry.item_id)
        .map_err(|e| service_failure(&e))?
        .filter(|binding| binding.provider == route.provider);
    if let Some(binding) = binding
        && let Some(at) = route
            .client
            .upload_offset(&task.person, &binding.upload_id)
            .await?
    {
        advance(caller, task, &entry.item_id, at)?;
        if at >= entry.size {
            finish(caller, route, task, &entry.item_id, &binding.upload_id).await?;
        }
        return Ok(at);
    }
    let start = UploadStart {
        path: entry.path.clone(),
        size: entry.size,
        checksum: entry.checksum.clone(),
        taken_at: entry.taken_at,
        live_video_checksum: None,
    };
    let created = route.client.start_upload(&task.person, &start).await?;
    store
        .bind_upload(
            &task.person,
            task.id,
            &entry.item_id,
            &UploadBinding {
                provider: route.provider.clone(),
                upload_id: created.upload_id.clone(),
            },
        )
        .map_err(|e| service_failure(&e))?;
    advance(caller, task, &entry.item_id, created.upload_offset)?;
    if created.upload_offset >= entry.size {
        finish(caller, route, task, &entry.item_id, &created.upload_id).await?;
    }
    Ok(created.upload_offset)
}

/// 让提供方收尾，这一项记完成；检查和上传之间别处传了同一张的，提供方说重复，记跳过，不算失败
/// （ADR-0113 第 9 节）。
async fn finish(
    caller: &Caller,
    route: &ProviderRoute,
    task: &MediaTask,
    item: &str,
    upload_id: &str,
) -> Reply<()> {
    let finished = route.client.finish_upload(&task.person, upload_id).await?;
    let tasks = &caller.media.tasks;
    let recorded = if finished.duplicate {
        tasks.skip_item(task.id, item)
    } else {
        tasks.finish_item(task.id, item, &route.provider)
    };
    recorded.map(|_| ()).map_err(|e| service_failure(&e))
}

fn advance(caller: &Caller, task: &MediaTask, item: &str, at: u64) -> Reply<()> {
    caller
        .media
        .tasks
        .advance_item(task.id, item, at)
        .map(|_| ())
        .map_err(|e| service_failure(&e))
}

pub(super) async fn upload_write(
    State(gateway): State<Arc<Gateway>>,
    Extension(caller): Extension<Caller>,
    Path((id, item)): Path<(String, String)>,
    headers: HeaderMap,
    body: Body,
) -> Response {
    let task = match parse_task(&id).and_then(|task| own_task(&caller, task)) {
        Ok(task) => task,
        Err(failure) => return reply(&failure),
    };
    let written = write_upload(&gateway, &caller, &task, &item, &headers, body).await;
    match written {
        Ok((status, at)) => offset(status, at),
        Err(failure) if failure.code == MediaFailureCode::Protocol => reply(&failure),
        Err(failure) => fail_task(&caller, &task, failure),
    }
}

async fn write_upload(
    gateway: &Gateway,
    caller: &Caller,
    task: &MediaTask,
    item: &str,
    headers: &HeaderMap,
    body: Body,
) -> Reply<(StatusCode, u64)> {
    let entry = upload_item(task, item)?;
    let at = resume_at(headers)?;
    let route = upload_route(gateway, caller, task).await?;
    let binding = caller
        .media
        .store
        .upload_binding(&task.person, task.id, item)
        .map_err(|e| service_failure(&e))?
        .filter(|binding| binding.provider == route.provider)
        .ok_or_else(|| {
            failure(
                MediaFailureCode::Protocol,
                "这一项还没开始上传，先 POST 同一个地址",
            )
        })?;
    let patched = route
        .client
        .patch_upload(&task.person, &binding.upload_id, at, pass(headers), body)
        .await;
    let outcome = match patched {
        // 提供方那边这个上传过期了：原地重开一个，回 409 带新偏移，设备照 409 的约定从那里续，任务不失败。
        Err(gone) if gone.code == MediaFailureCode::MediaGone => {
            let reopened = open_upload(caller, &route, task, &entry).await?;
            return Ok((StatusCode::CONFLICT, reopened));
        }
        other => other?,
    };
    advance(caller, task, item, outcome.offset)?;
    if outcome.conflict {
        return Ok((StatusCode::CONFLICT, outcome.offset));
    }
    if outcome.offset >= entry.size {
        finish(caller, &route, task, item, &binding.upload_id).await?;
    }
    Ok((StatusCode::OK, outcome.offset))
}

/// 这一段从哪儿续：`Content-Range` 的起点，没带就看 `Upload-Offset`。
fn resume_at(headers: &HeaderMap) -> Reply<u64> {
    let text = |name: HeaderName| headers.get(name).and_then(|v| v.to_str().ok());
    let from_range = text(header::CONTENT_RANGE).and_then(|value| {
        let spec = value.strip_prefix("bytes")?.trim_start();
        spec.split('-').next()?.trim().parse().ok()
    });
    from_range
        .or_else(|| {
            text(HeaderName::from_static("upload-offset"))?
                .trim()
                .parse()
                .ok()
        })
        .ok_or_else(|| {
            failure(
                MediaFailureCode::Protocol,
                format!("续写要带 Content-Range 或 {UPLOAD_OFFSET_HEADER}"),
            )
        })
}

pub(super) async fn content(
    State(gateway): State<Arc<Gateway>>,
    Extension(caller): Extension<Caller>,
    Path((id, item)): Path<(String, String)>,
    headers: HeaderMap,
) -> Response {
    let fetched = async {
        let task = own_task(&caller, parse_task(&id)?)?;
        let TaskTarget::Download(target) = &task.target else {
            return Err(failure(MediaFailureCode::Protocol, "这不是下载任务"));
        };
        if task.kind != TaskKind::Download || target.id != item {
            return Err(failure(
                MediaFailureCode::MediaGone,
                format!("任务里没有 {item} 这一项"),
            ));
        }
        let known = caller
            .media
            .store
            .task_provider(&task.person, task.id)
            .map_err(|e| service_failure(&e))?;
        let providers = &caller.media.providers;
        let route = match known {
            Some(provider) => {
                download_route(&gateway, &caller, &task, &target.kind, &provider).await?
            }
            None => {
                let route = providers
                    .route(
                        &task.person,
                        caller.allowed(&gateway),
                        &MediaOperation::Download,
                        Some(&target.kind),
                        target.source.as_deref(),
                    )
                    .await?;
                let bound = caller
                    .media
                    .store
                    .bind_task_provider(&task.person, task.id, &route.provider)
                    .map_err(|e| service_failure(&e))?;
                if bound == route.provider {
                    route
                } else {
                    download_route(&gateway, &caller, &task, &target.kind, &bound).await?
                }
            }
        };
        // 设备来取字节就是开始下了：排队的进 `Running`，之后报进度、写完都从这里接着走。
        caller
            .media
            .tasks
            .begin(task.id, Some(&route.provider))
            .map_err(|e| service_failure(&e))?;
        route
            .client
            .forward(
                &task.person,
                Method::GET,
                &item_download_path(&item),
                pass(&headers),
                Body::empty(),
            )
            .await
    };
    match fetched.await {
        Ok(response) => response,
        Err(failure) => reply(&failure),
    }
}

/// 下载任务记下的那家，按下载和条目的种类核对。
async fn download_route(
    gateway: &Gateway,
    caller: &Caller,
    task: &MediaTask,
    kind: &MediaKind,
    provider: &str,
) -> Reply<ProviderRoute> {
    caller
        .media
        .providers
        .route_to(
            &task.person,
            caller.allowed(gateway),
            provider,
            &MediaOperation::Download,
            Some(kind),
        )
        .await
}

/// 取封面的查询串：`kind`、`source` 用来选提供方，`type`、`width` 原样转给它。
#[derive(Deserialize)]
pub(super) struct ArtworkRequest {
    kind: MediaKind,
    #[serde(default)]
    source: Option<String>,
    #[serde(rename = "type", default)]
    artwork: Option<ArtworkKind>,
    #[serde(default)]
    width: Option<u32>,
}

/// 按种类列条目，转给这个种类的提供方。
pub(super) async fn items(
    State(gateway): State<Arc<Gateway>>,
    Extension(caller): Extension<Caller>,
    MediaQuery(query): MediaQuery<ItemsQuery>,
) -> Response {
    let listed = async {
        let kind = query
            .kind
            .clone()
            .ok_or_else(|| failure(MediaFailureCode::Protocol, "列条目要带 kind"))?;
        let route = caller
            .media
            .providers
            .route(
                &caller.person,
                caller.allowed(&gateway),
                &MediaOperation::List,
                Some(&kind),
                query.source.as_deref(),
            )
            .await?;
        let upstream = ItemsQuery {
            source: None,
            ..query.clone()
        };
        let mut page = route.client.items(&caller.person, &upstream).await?;
        // 来源由核心填，提供方自己填的不算数。提供方的封面地址手机连不到，也不跟着它走：有图就换成
        // 核心的封面路由，核心再按契约的 `GET /items/{id}/artwork` 去取。
        for item in &mut page.items {
            item.source = Some(route.provider.clone());
            if item.artwork_url.is_some() {
                item.artwork_url = Some(api::item_artwork_path(item, &ArtworkQuery::default()));
            }
        }
        Ok(page)
    };
    match listed.await {
        Ok(page) => Json(page).into_response(),
        Err(failure) => reply(&failure),
    }
}

/// 封面、海报、缩略图的字节，转给这个种类的提供方。
pub(super) async fn artwork(
    State(gateway): State<Arc<Gateway>>,
    Extension(caller): Extension<Caller>,
    Path(item): Path<String>,
    MediaQuery(request): MediaQuery<ArtworkRequest>,
    headers: HeaderMap,
) -> Response {
    let fetched = async {
        let route = caller
            .media
            .providers
            .route(
                &caller.person,
                caller.allowed(&gateway),
                &MediaOperation::List,
                Some(&request.kind),
                request.source.as_deref(),
            )
            .await?;
        let size = ArtworkQuery {
            artwork: request.artwork,
            width: request.width,
        };
        route
            .client
            .forward(
                &caller.person,
                Method::GET,
                &item_artwork_path(&item, &size),
                pass(&headers),
                Body::empty(),
            )
            .await
    };
    match fetched.await {
        Ok(response) => response,
        Err(failure) => reply(&failure),
    }
}
