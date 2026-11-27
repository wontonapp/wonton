//! 播放会话：起播（向提供方要计划）、列、读、控制（切轨、换曲时重取计划）、报位置、停止。
//!
//! 给客户端的计划里 `stream` 换成核心的 `/sessions/{id}/stream`，提供方原址不下发；位置和停止核心先落、
//! 再转给提供方，转不过去只记日志：会话的事实源在核心，提供方那边的播放记录晚一步补上。

use std::sync::Arc;

use axum::Extension;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::response::{IntoResponse, Json, Response};
use hyper::StatusCode;
use wonton_protocol::media::api::{SessionList, session_stream_path};
use wonton_protocol::media::provider::{PlaybackProgress, PlaybackStopped};
use wonton_protocol::media::{
    ControlOp, MediaAction, MediaFailure, MediaFailureCode, MediaItemRef, MediaOperation,
    PlaybackRequest, PlaybackSession, PlaybackSessionId, ProviderStream, TrackKind,
};

use super::{Caller, reply, service_failure};
use crate::gateway::Gateway;
use crate::media::provider::ProviderRoute;
use crate::plugin::media_source::failure;

type Reply<T> = Result<T, MediaFailure>;

fn json<T: serde::de::DeserializeOwned>(body: &Bytes) -> Reply<T> {
    serde_json::from_slice(body).map_err(|e| {
        failure(
            MediaFailureCode::Protocol,
            format!("请求体不是约定的形状：{e}"),
        )
    })
}

fn respond(result: Reply<PlaybackSession>) -> Response {
    match result {
        Ok(session) => Json(present(session)).into_response(),
        Err(failure) => reply(&failure),
    }
}

/// 给客户端看的会话：流换成核心路由，原址不下发。
pub(super) fn present(mut session: PlaybackSession) -> PlaybackSession {
    if let Some(plan) = &mut session.plan {
        let path = session_stream_path(session.id);
        plan.stream = match plan.stream {
            ProviderStream::Direct(_) => ProviderStream::Direct(path),
            ProviderStream::Transcoded(_) => ProviderStream::Transcoded(path),
            ProviderStream::Other(_) => ProviderStream::Other(path),
        };
        plan.origin_stream = None;
    }
    session
}

pub(super) fn parse_session(raw: &str) -> Reply<PlaybackSessionId> {
    raw.parse()
        .map_err(|_| failure(MediaFailureCode::MediaGone, format!("没有会话 {raw}")))
}

/// 这个人的会话；别人的当没有。
pub(super) fn own_session(caller: &Caller, raw: &str) -> Reply<PlaybackSession> {
    let id = parse_session(raw)?;
    let stored = caller
        .media
        .store
        .stored_session(id)
        .map_err(|e| service_failure(&e))?;
    if stored.session.person != caller.person {
        return Err(failure(
            MediaFailureCode::MediaGone,
            format!("没有会话 {id}"),
        ));
    }
    Ok(stored.session)
}

/// 会话记下的提供方。
pub(super) async fn session_route(
    gateway: &Gateway,
    caller: &Caller,
    session: &PlaybackSession,
) -> Reply<ProviderRoute> {
    let provider = caller
        .media
        .store
        .session_provider(session.id)
        .map_err(|e| service_failure(&e))?
        .ok_or_else(|| failure(MediaFailureCode::MediaGone, "这条会话没记提供方"))?;
    caller
        .media
        .providers
        .route_to(
            &caller.person,
            caller.allowed(gateway),
            &provider,
            &MediaOperation::Playback,
            Some(&session.item.kind),
        )
        .await
}

/// 按条目自己的来源选提供方（没带按主人的默认、只有一家的规则）：队列是相册、音乐、影视共用的一条，
/// 下一项可能来自另一家，不能拿会话起播那家去问。
pub(super) async fn item_route(
    gateway: &Gateway,
    caller: &Caller,
    person: &str,
    item: &MediaItemRef,
) -> Reply<ProviderRoute> {
    caller
        .media
        .providers
        .route(
            person,
            caller.allowed(gateway),
            &MediaOperation::Playback,
            Some(&item.kind),
            item.source.as_deref(),
        )
        .await
}

pub(super) async fn start(
    State(gateway): State<Arc<Gateway>>,
    Extension(caller): Extension<Caller>,
    body: Bytes,
) -> Response {
    let started = async {
        let mut request: PlaybackRequest = json(&body)?;
        request.person = caller.person(&request.person)?;
        request.device = caller.device(&request.device)?;
        let route = caller
            .media
            .providers
            .route(
                &request.person,
                caller.allowed(&gateway),
                &MediaOperation::Playback,
                Some(&request.item.kind),
                request.item.source.as_deref(),
            )
            .await?;
        request.item.source = Some(route.provider.clone());
        let plan = route.client.playback(&request.person, &request).await?;
        let (session, _taken_over) = caller
            .media
            .sessions
            .start(&request, plan)
            .map_err(|e| service_failure(&e))?;
        caller
            .media
            .store
            .bind_session_route(session.id, &route.provider, &request.profile)
            .map_err(|e| service_failure(&e))?;
        Ok(session)
    };
    match started.await {
        Ok(session) => (StatusCode::CREATED, Json(present(session))).into_response(),
        Err(failure) => reply(&failure),
    }
}

pub(super) async fn list(Extension(caller): Extension<Caller>) -> Response {
    match caller.media.store.sessions(Some(&caller.person)) {
        Ok(sessions) => Json(SessionList {
            sessions: sessions.into_iter().map(present).collect(),
        })
        .into_response(),
        Err(e) => reply(&service_failure(&e)),
    }
}

pub(super) async fn read(Extension(caller): Extension<Caller>, Path(id): Path<String>) -> Response {
    respond(own_session(&caller, &id))
}

pub(super) async fn control(
    State(gateway): State<Arc<Gateway>>,
    Extension(caller): Extension<Caller>,
    Path(id): Path<String>,
    body: Bytes,
) -> Response {
    let controlled = async {
        let session = own_session(&caller, &id)?;
        let ctl = match json::<MediaAction>(&body)? {
            MediaAction::Control {
                session: target,
                ctl,
            } if target == session.id => ctl,
            _ => {
                return Err(failure(
                    MediaFailureCode::Protocol,
                    "请求体应该是这条会话的 control 动作",
                ));
            }
        };
        let replan = matches!(ctl, ControlOp::SetAudio(_) | ControlOp::SetSubtitle(_));
        let updated = caller
            .media
            .sessions
            .control(session.id, ctl.clone())
            .map_err(|e| service_failure(&e))?;
        let updated = if !(replan || updated.plan.is_none()) || updated.stop_reason.is_some() {
            updated
        } else {
            replan_session(&gateway, &caller, &updated, &ctl).await?
        };
        // 换曲动了队列的指针，同样推给提供方。
        if matches!(ctl, ControlOp::Next | ControlOp::Prev) {
            let queue = caller
                .media
                .sessions
                .queue(&caller.person)
                .map_err(|e| service_failure(&e))?;
            super::queue::push(&gateway, &caller, &queue).await;
        }
        Ok(updated)
    };
    respond(controlled.await)
}

/// 切轨、换曲后重取计划：同一个设备档、当前位置、这次选的轨。切轨问会话记下的那家；换曲按新那一项的
/// 来源选，换了家就把会话记到新的那家，之后的流、位置、停止都发给它。取不到就报错，客户端继续用旧计划。
async fn replan_session(
    gateway: &Gateway,
    caller: &Caller,
    session: &PlaybackSession,
    ctl: &ControlOp,
) -> Reply<PlaybackSession> {
    let profile = caller
        .media
        .store
        .session_profile(session.id)
        .map_err(|e| service_failure(&e))?
        .ok_or_else(|| failure(MediaFailureCode::MediaGone, "这条会话没记设备档"))?;
    let selected = |kind: TrackKind| {
        session.plan.as_ref().and_then(|plan| {
            let tracks = if kind == TrackKind::Audio {
                &plan.audio
            } else {
                &plan.subtitles
            };
            tracks.iter().find(|t| t.selected).map(|t| t.index)
        })
    };
    let (audio, subtitle) = match ctl {
        ControlOp::SetAudio(index) => (Some(*index), selected(TrackKind::Subtitle)),
        ControlOp::SetSubtitle(index) => (selected(TrackKind::Audio), *index),
        _ => (None, None),
    };
    let request = PlaybackRequest {
        position_ms: session.position_ms,
        audio,
        subtitle,
        item: session.item.clone(),
        device: session.device.clone(),
        person: session.person.clone(),
        profile,
    };
    let route = if matches!(ctl, ControlOp::SetAudio(_) | ControlOp::SetSubtitle(_)) {
        session_route(gateway, caller, session).await?
    } else {
        let route = item_route(gateway, caller, &session.person, &session.item).await?;
        caller
            .media
            .store
            .bind_session_route(session.id, &route.provider, &request.profile)
            .map_err(|e| service_failure(&e))?;
        route
    };
    let plan = route.client.playback(&session.person, &request).await?;
    caller
        .media
        .sessions
        .set_plan(session.id, plan)
        .map_err(|e| service_failure(&e))
}

pub(super) async fn progress(
    State(gateway): State<Arc<Gateway>>,
    Extension(caller): Extension<Caller>,
    Path(id): Path<String>,
    body: Bytes,
) -> Response {
    let reported = async {
        let session = own_session(&caller, &id)?;
        let report: PlaybackProgress = json(&body)?;
        let landed = caller
            .media
            .sessions
            .progress(session.id, report.position_ms, report.wall_clock)
            .map_err(|e| service_failure(&e))?;
        if landed && let Some(plan) = &session.plan {
            let route = session_route(&gateway, &caller, &session).await;
            let sent = match route {
                Ok(route) => {
                    route
                        .client
                        .progress(&session.person, &plan.session, &report)
                        .await
                }
                Err(e) => Err(e),
            };
            if let Err(e) = sent {
                tracing::warn!(session = %session.id, code = ?e.code, error = %e.message, "播放位置没转给提供方");
            }
        }
        Ok(())
    };
    match reported.await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(failure) => reply(&failure),
    }
}

pub(super) async fn stop(
    State(gateway): State<Arc<Gateway>>,
    Extension(caller): Extension<Caller>,
    Path(id): Path<String>,
    body: Bytes,
) -> Response {
    let stopped = async {
        let session = own_session(&caller, &id)?;
        let request: PlaybackStopped = json(&body)?;
        let updated = caller
            .media
            .sessions
            .stop(session.id, request.position_ms, request.reason.clone())
            .map_err(|e| service_failure(&e))?;
        caller.media.queued.forget(session.id);
        if let Some(plan) = &session.plan {
            let report = PlaybackStopped {
                position_ms: updated.position_ms,
                reason: request.reason,
            };
            let sent = match session_route(&gateway, &caller, &session).await {
                Ok(route) => {
                    route
                        .client
                        .stopped(&session.person, &plan.session, &report)
                        .await
                }
                Err(e) => Err(e),
            };
            if let Err(e) = sent {
                tracing::warn!(session = %session.id, code = ?e.code, error = %e.message, "停播没转给提供方");
            }
        }
        Ok(())
    };
    match stopped.await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(failure) => reply(&failure),
    }
}
