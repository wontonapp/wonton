//! 播放的流（ADR-0113 第 2 节“字节路径”）：核心按提供方计划里的流地址转给它，不落盘、不改写播放列表。
//!
//! - `GET /sessions/{id}/stream/{文件名}`：计划里的流。给客户端的计划里 `stream` 就是这个地址，文件名取提供方
//!   流地址的最后一段，查询串不下发（可能带提供方的令牌），取流本身时核心用计划里的。
//! - `GET /sessions/{id}/stream/{rest}`：流旁边的一段。HLS 主列表里的子列表、分片写相对地址，播放器按上面那个
//!   地址解析出来；核心把 `rest` 接回提供方流地址的目录，查询串用这次请求自己的。只给 `/sessions/{id}/stream`
//!   不够：相对地址会解析到 `/sessions/{id}/` 下面去。
//! - `GET /sessions/{id}/items/{item}/stream`（后面同样可接 `/{rest}`）：队列里另一项的流，无缝播放和预取用。
//!
//! 提供方给的流地址只取路径和查询串，按它的服务地址去取：核心只连这个提供方，不跟着地址去别处。

use std::sync::Arc;

use axum::Extension;
use axum::body::Body;
use axum::extract::{Path, RawQuery, State};
use axum::response::Response;
use hyper::Method;
use hyper::header::HeaderMap;
use percent_encoding::percent_decode_str;
use wonton_protocol::media::api::session_stream_path;
use wonton_protocol::media::provider::segment;
use wonton_protocol::media::{
    MediaFailure, MediaFailureCode, MediaItemRef, PlaybackRequest, PlaybackSession,
    PlaybackSessionId, ProviderStream,
};

use super::bytes::pass;
use super::sessions::{item_route, own_session, session_route};
use super::{Caller, reply, service_failure};
use crate::gateway::Gateway;
use crate::media::provider::ProviderRoute;
use crate::media::session_service::unix_now_ms;
use crate::plugin::media_source::failure;

type Reply<T> = Result<T, MediaFailure>;

/// 给客户端的流地址：`/sessions/{id}/stream/{文件名}`，不带查询串。流地址以 `/` 结尾、取不出文件名的，
/// 给 `/sessions/{id}/stream`。
pub(super) fn presented(id: PlaybackSessionId, stream: &ProviderStream) -> ProviderStream {
    let base = session_stream_path(id);
    let name = provider_path(stream).ok().and_then(|path| file_name(&path));
    let path = match name {
        Some(name) => format!("{base}/{}", segment(&name)),
        None => base,
    };
    match stream {
        ProviderStream::Direct(_) => ProviderStream::Direct(path),
        ProviderStream::Transcoded(_) => ProviderStream::Transcoded(path),
        ProviderStream::Other(_) => ProviderStream::Other(path),
    }
}

pub(super) async fn stream(
    State(gateway): State<Arc<Gateway>>,
    Extension(caller): Extension<Caller>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    plan_stream(&gateway, &caller, &id, None, &headers).await
}

pub(super) async fn stream_part(
    State(gateway): State<Arc<Gateway>>,
    Extension(caller): Extension<Caller>,
    Path((id, rest)): Path<(String, String)>,
    RawQuery(query): RawQuery,
    headers: HeaderMap,
) -> Response {
    let part = Part {
        rest: &rest,
        query: query.as_deref(),
    };
    plan_stream(&gateway, &caller, &id, Some(part), &headers).await
}

async fn plan_stream(
    gateway: &Gateway,
    caller: &Caller,
    id: &str,
    part: Option<Part<'_>>,
    headers: &HeaderMap,
) -> Response {
    let fetched = async {
        let session = own_session(caller, id)?;
        let plan = session
            .plan
            .as_ref()
            .ok_or_else(|| failure(MediaFailureCode::MediaGone, "这条会话现在没有计划"))?;
        let path = beside(provider_path(&plan.stream)?, part)?;
        let route = session_route(gateway, caller, &session).await?;
        route
            .client
            .forward(
                &session.person,
                Method::GET,
                &path,
                pass(headers),
                Body::empty(),
            )
            .await
    };
    match fetched.await {
        Ok(response) => response,
        Err(failure) => reply(&failure),
    }
}

/// 队列里另一项的流（无缝播放、预取）：这一项必须在这个人的队列里，按会话起播时的设备档、向这一项
/// 自己的来源要它的计划，取计划里的流。不改会话，也不进播放记录。
pub(super) async fn queued_stream(
    State(gateway): State<Arc<Gateway>>,
    Extension(caller): Extension<Caller>,
    Path((id, item)): Path<(String, String)>,
    headers: HeaderMap,
) -> Response {
    queued(&gateway, &caller, (&id, &item), None, &headers).await
}

pub(super) async fn queued_stream_part(
    State(gateway): State<Arc<Gateway>>,
    Extension(caller): Extension<Caller>,
    Path((id, item, rest)): Path<(String, String, String)>,
    RawQuery(query): RawQuery,
    headers: HeaderMap,
) -> Response {
    let part = Part {
        rest: &rest,
        query: query.as_deref(),
    };
    queued(&gateway, &caller, (&id, &item), Some(part), &headers).await
}

async fn queued(
    gateway: &Gateway,
    caller: &Caller,
    (id, item): (&str, &str),
    part: Option<Part<'_>>,
    headers: &HeaderMap,
) -> Response {
    let fetched = async {
        let session = own_session(caller, id)?;
        let entry = queued_entry(caller, &session, item)?;
        let route = item_route(gateway, caller, &session.person, &entry).await?;
        let path = match caller.media.queued.get(session.id, item, unix_now_ms()) {
            Some(path) => path,
            None => queued_plan(caller, &route, &session, entry).await?,
        };
        let path = beside(path, part)?;
        route
            .client
            .forward(
                &session.person,
                Method::GET,
                &path,
                pass(headers),
                Body::empty(),
            )
            .await
    };
    match fetched.await {
        Ok(response) => response,
        Err(failure) => reply(&failure),
    }
}

fn queued_entry(caller: &Caller, session: &PlaybackSession, item: &str) -> Reply<MediaItemRef> {
    let queue = caller
        .media
        .sessions
        .queue(&session.person)
        .map_err(|e| service_failure(&e))?;
    queue
        .entries
        .into_iter()
        .find(|entry| entry.id == item)
        .ok_or_else(|| failure(MediaFailureCode::MediaGone, format!("队列里没有 {item}")))
}

async fn queued_plan(
    caller: &Caller,
    route: &ProviderRoute,
    session: &PlaybackSession,
    entry: MediaItemRef,
) -> Reply<String> {
    let item = entry.id.clone();
    let profile = caller
        .media
        .store
        .session_profile(session.id)
        .map_err(|e| service_failure(&e))?
        .ok_or_else(|| failure(MediaFailureCode::MediaGone, "这条会话没记设备档"))?;
    let request = PlaybackRequest {
        position_ms: 0,
        audio: None,
        subtitle: None,
        item: entry,
        device: session.device.clone(),
        person: session.person.clone(),
        profile,
    };
    let plan = route.client.playback(&session.person, &request).await?;
    let path = provider_path(&plan.stream)?;
    caller
        .media
        .queued
        .put(session.id, &item, path.clone(), plan.expires_at);
    Ok(path)
}

/// 客户端在流地址后面接的那一段（axum 已解码），和这次请求自己的查询串。
#[derive(Clone, Copy)]
struct Part<'a> {
    rest: &'a str,
    query: Option<&'a str>,
}

/// 按客户端接的那一段换成提供方的路径。正好是流的文件名：就是流本身，原样用计划里的路径和查询串。
/// 其余的接到流地址的目录后面，逐段重新转义，`.`、`..`、空段一律拒：拼出来的路径不会跑到流地址所在
/// 目录的上面去。
fn beside(stream: String, part: Option<Part<'_>>) -> Reply<String> {
    let Some(Part { rest, query }) = part else {
        return Ok(stream);
    };
    if file_name(&stream).as_deref() == Some(rest) {
        return Ok(stream);
    }
    let path = stream
        .split_once('?')
        .map_or(stream.as_str(), |(path, _)| path);
    let dir = path.rfind('/').map_or("/", |end| &path[..=end]);
    let mut segments = Vec::new();
    for piece in rest.split('/') {
        if matches!(piece, "" | "." | "..") {
            return Err(failure(
                MediaFailureCode::Protocol,
                format!("流的子路径不合法：{rest}"),
            ));
        }
        segments.push(segment(piece));
    }
    let mut joined = format!("{dir}{}", segments.join("/"));
    if let Some(query) = query.filter(|query| !query.is_empty()) {
        joined.push('?');
        joined.push_str(query);
    }
    Ok(joined)
}

/// 流地址最后一段解码后的文件名；以 `/` 结尾、解码出 `.`、`..` 或 `/` 的没有。
fn file_name(path: &str) -> Option<String> {
    let path = path.split_once('?').map_or(path, |(path, _)| path);
    let last = &path[path.rfind('/')? + 1..];
    let name = percent_decode_str(last).decode_utf8().ok()?;
    (!matches!(&*name, "" | "." | "..") && !name.contains('/')).then(|| name.into_owned())
}

/// 提供方给的流地址只取路径和查询串。
fn provider_path(stream: &ProviderStream) -> Reply<String> {
    let raw = match stream {
        ProviderStream::Direct(url)
        | ProviderStream::Transcoded(url)
        | ProviderStream::Other(url) => url,
    };
    let uri: hyper::Uri = raw.parse().map_err(|e| {
        failure(
            MediaFailureCode::Protocol,
            format!("计划里的流地址不合法：{e}"),
        )
    })?;
    uri.path_and_query()
        .map(|pq| pq.as_str().to_owned())
        .filter(|pq| pq.starts_with('/'))
        .ok_or_else(|| failure(MediaFailureCode::Protocol, "计划里的流地址没有路径"))
}
