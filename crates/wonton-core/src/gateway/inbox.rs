//! 收件箱接口（ADR-0119）：列表、详情、作答、已读、稍后、取消、页面和 worker 事件。
//!
//! 和确认卡片同一道门：主人令牌，或者经隧道的已配对设备。确认卡片投影成 `confirm` 条目，回答仍回到确认表。

use std::sync::Arc;
use std::time::Duration;

use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use hyper::{HeaderMap, StatusCode};
use serde::Deserialize;
use wonton_protocol::inbox::{
    INBOX_WAIT_SECS, InboxAnswerInput, InboxItem, InboxPageView, InboxView, InboxWorkerAnswer,
    InboxWorkerEvent, InboxWorkerEventKind, InboxWorkerResolve,
};
use wonton_protocol::{
    Answer, ConfirmAction, ConfirmPayload, InboxAnswerAction, InboxLevel, InboxPayload,
    InboxRecipient, InboxRisk, InboxSender, InboxSenderKind, InboxSource, InboxStatus, InboxType,
    NoticePayload, RecapPayload,
};

use super::api::{error_response, languages, plugin_denied, requester};
use super::{Gateway, Person, TunnelPeer};
use crate::inbox::{CreateInput, Inbox, InboxError, policy};

pub(super) fn routes() -> axum::Router<Arc<Gateway>> {
    use axum::routing::{get, post};
    axum::Router::new()
        .route(wonton_protocol::INBOX_PATH, get(list))
        .route(
            &format!("{}/{{id}}", wonton_protocol::INBOX_PATH),
            get(detail),
        )
        .route(
            &format!("{}/{{id}}/answer", wonton_protocol::INBOX_PATH),
            post(answer),
        )
        .route(
            &format!("{}/{{id}}/acknowledge", wonton_protocol::INBOX_PATH),
            post(acknowledge),
        )
        .route(
            &format!("{}/{{id}}/read", wonton_protocol::INBOX_PATH),
            post(read),
        )
        .route(
            &format!("{}/{{id}}/cancel", wonton_protocol::INBOX_PATH),
            post(cancel),
        )
        .route(
            &format!("{}/{{id}}/snooze", wonton_protocol::INBOX_PATH),
            post(snooze),
        )
        .route(
            &format!("{}/{{id}}/page", wonton_protocol::INBOX_PATH),
            get(page),
        )
        .route(wonton_protocol::INBOX_WORKERS_PATH, post(worker_event))
        .route(
            &format!("{}/{{worker_id}}", wonton_protocol::INBOX_WORKERS_PATH),
            get(worker_answer),
        )
        .route(
            &format!(
                "{}/{{worker_id}}/resolve",
                wonton_protocol::INBOX_WORKERS_PATH
            ),
            post(worker_resolve),
        )
}

#[allow(clippy::result_large_err)]
fn inbox(gateway: &Gateway) -> Result<Arc<Inbox>, Response> {
    gateway
        .mcp
        .as_ref()
        .and_then(|endpoint| endpoint.bus.inbox())
        .ok_or_else(|| {
            error_response(
                StatusCode::SERVICE_UNAVAILABLE,
                "service_unavailable",
                "收件箱没开",
            )
        })
}

fn fail(error: InboxError) -> Response {
    error_response(
        StatusCode::from_u16(error.status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
        error.kind(),
        &error.to_string(),
    )
}

#[allow(clippy::result_large_err)]
fn person(
    gateway: &Gateway,
    peer: Option<Extension<TunnelPeer>>,
    headers: &HeaderMap,
) -> Result<Person, Response> {
    if let Some(denied) = plugin_denied(gateway, peer, headers) {
        return Err(denied);
    }
    requester(gateway, peer, headers).map_err(|response| *response)
}

fn is_admin(gateway: &Gateway, viewer: &str) -> bool {
    let owner = gateway.owner.username();
    viewer == owner
        || gateway
            .mcp
            .as_ref()
            .is_some_and(|endpoint| endpoint.bus.plugin_admins().iter().any(|a| a == viewer))
}

fn confirm_item(
    gateway: &Gateway,
    card: wonton_protocol::ConfirmationCard,
    keeper: &str,
    now: u64,
) -> InboxItem {
    let _ = gateway;
    crate::inbox::project_confirmation(&card, Some(keeper), now)
}

#[derive(Deserialize)]
struct ListQuery {
    #[serde(default)]
    after: Option<u64>,
}

async fn list(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
    Query(query): Query<ListQuery>,
) -> Response {
    let inbox = match inbox(&gateway) {
        Ok(inbox) => inbox,
        Err(response) => return response,
    };
    let person = match person(&gateway, peer, &headers) {
        Ok(person) => person,
        Err(response) => return response,
    };
    let viewer = person.username.clone();
    let keeper = if person.is_owner() {
        gateway.owner.username().to_owned()
    } else {
        viewer.clone()
    };
    // 列表的版本是收件箱自己的版本加确认表的版本（确认卡片投影进来，见下面）：等的时候两边任何一边变了
    // 都要醒，比较也要用同一个和，不然确认卡片变了要白等到超时才返回。
    let revision = || {
        inbox.revision().saturating_add(
            gateway
                .mcp
                .as_ref()
                .map(|e| e.bus.confirmations().revision())
                .unwrap_or_default(),
        )
    };
    if let Some(after) = query.after
        && after >= revision()
    {
        let mut cards = gateway
            .mcp
            .as_ref()
            .map(|e| e.bus.confirmations().subscribe());
        let mut own = inbox.subscribe();
        let deadline = tokio::time::Instant::now() + Duration::from_secs(INBOX_WAIT_SECS);
        while revision() <= after {
            let changed = tokio::select! {
                _ = own.changed() => true,
                _ = async {
                    match cards.as_mut() {
                        Some(cards) => { let _ = cards.changed().await; }
                        None => std::future::pending::<()>().await,
                    }
                } => true,
                _ = tokio::time::sleep_until(deadline) => false,
            };
            if !changed {
                break;
            }
        }
    }
    let now = crate::inbox::now_ms();
    let revision = revision();
    let mut list = inbox.list(&viewer, now);
    if let Some(endpoint) = &gateway.mcp {
        let confirmations = endpoint.bus.confirmations();
        let recipient = (!person.is_owner()).then_some(viewer.as_str());
        let (pending, finished, _) = confirmations.list(recipient);
        for card in pending.into_iter().chain(finished) {
            if !card_allowed_for(&gateway, &card, &viewer) {
                continue;
            }
            let item = confirm_item(&gateway, card, &keeper, now);
            let summary = crate::inbox::summarize_item(&item, &viewer);
            if item.status == InboxStatus::Pending
                && (item.kind == InboxType::Choice || item.kind == InboxType::Confirm)
            {
                list.counts.needing = list.counts.needing.saturating_add(1);
            }
            list.items.push(summary);
        }
    }
    crate::inbox::sort_list(&mut list.items);
    list.revision = revision;
    Json(list).into_response()
}

fn card_allowed_for(
    gateway: &Gateway,
    card: &wonton_protocol::ConfirmationCard,
    viewer: &str,
) -> bool {
    match card.audience {
        wonton_protocol::CardAudience::Admins => is_admin(gateway, viewer),
        _ => true,
    }
}

async fn detail(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    let inbox = match inbox(&gateway) {
        Ok(inbox) => inbox,
        Err(response) => return response,
    };
    let person = match person(&gateway, peer, &headers) {
        Ok(person) => person,
        Err(response) => return response,
    };
    let now = crate::inbox::now_ms();
    let viewer = person.username.clone();
    let keeper = if person.is_owner() {
        gateway.owner.username().to_owned()
    } else {
        viewer.clone()
    };
    if let Some(card_id) = crate::inbox::confirmation_id(&id) {
        let Some(endpoint) = &gateway.mcp else {
            return fail(InboxError::ServiceUnavailable);
        };
        if let Some(card) = endpoint.bus.confirmations().card(card_id) {
            if !card_allowed_for(&gateway, &card, &viewer) {
                return fail(InboxError::NotYours);
            }
            let item = confirm_item(&gateway, card, &keeper, now);
            let pending = matches!(item.status, InboxStatus::Pending);
            let view = InboxView {
                item,
                page: None,
                can_answer: pending,
                can_reedit: false,
                read_by_me: Some(viewer),
            };
            return Json(view).into_response();
        }
    }
    match inbox.view(&viewer, &id, now) {
        Ok(mut view) => {
            if let Some(page) = view.page.take() {
                let token = inbox.page_tokens().issue(&id, now);
                view.page = Some(InboxPageView {
                    url: wonton_protocol::inbox_page_path(&id, &token),
                    network_domains: page.network_domains,
                    csp: page.csp,
                });
            }
            Json(view).into_response()
        }
        Err(error) => fail(error),
    }
}

async fn answer(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    body: Bytes,
) -> Response {
    let inbox = match inbox(&gateway) {
        Ok(inbox) => inbox,
        Err(response) => return response,
    };
    let person = match person(&gateway, peer, &headers) {
        Ok(person) => person,
        Err(response) => return response,
    };
    let input: InboxAnswerInput = match serde_json::from_slice(&body) {
        Ok(input) => input,
        Err(error) => {
            return error_response(
                StatusCode::BAD_REQUEST,
                "invalid_input",
                &format!("答案不是合法 JSON：{error}"),
            );
        }
    };
    let now = crate::inbox::now_ms();
    if let Some(card_id) = crate::inbox::confirmation_id(&id) {
        let Some(endpoint) = &gateway.mcp else {
            return fail(InboxError::ServiceUnavailable);
        };
        if let Some(card) = endpoint.bus.confirmations().card(card_id) {
            let Some((answer, choice_id)) = confirm_answer_of(&card, &input) else {
                return error_response(
                    StatusCode::BAD_REQUEST,
                    "invalid_input",
                    "答案不是这张卡片的档",
                );
            };
            return match endpoint.bus.confirmations().answer_with_secrets_by(
                card_id,
                answer,
                choice_id.as_deref(),
                Default::default(),
                Default::default(),
                Vec::new(),
                &languages(&headers),
                Some(&person.username),
            ) {
                Ok(state) => {
                    let _ = state;
                    let item = confirm_item(&gateway, card, &person.username, now);
                    Json(InboxView {
                        item,
                        page: None,
                        can_answer: false,
                        can_reedit: false,
                        read_by_me: Some(person.username),
                    })
                    .into_response()
                }
                Err(error) => {
                    use crate::mcp::AnswerError;
                    let status = match error {
                        AnswerError::NotFound => StatusCode::NOT_FOUND,
                        AnswerError::Expired | AnswerError::AlreadyAnswered => StatusCode::CONFLICT,
                        AnswerError::UnknownAnswer | AnswerError::Secrets(_) => {
                            StatusCode::BAD_REQUEST
                        }
                    };
                    error_response(status, error.kind(), &error.to_string())
                }
            };
        }
    }
    match inbox.answer(&person.username, &id, input) {
        Ok(_outcome) => match inbox.view(&person.username, &id, now) {
            Ok(view) => Json(view).into_response(),
            Err(error) => fail(error),
        },
        Err(error) => fail(error),
    }
}

fn confirm_answer_of(
    card: &wonton_protocol::ConfirmationCard,
    input: &InboxAnswerInput,
) -> Option<(Answer, Option<String>)> {
    let choice = match input.action {
        InboxAnswerAction::Approve | InboxAnswerAction::Submit => {
            card.choices.iter().find(|choice| {
                matches!(
                    choice.answer,
                    Answer::Approve | Answer::ApproveWider | Answer::AllowWindow
                )
            })
        }
        InboxAnswerAction::Deny | InboxAnswerAction::Decline => card
            .choices
            .iter()
            .find(|choice| matches!(choice.answer, Answer::Reject)),
        InboxAnswerAction::Session => card
            .choices
            .iter()
            .find(|choice| choice.choice_id.as_deref() == Some("session")),
        InboxAnswerAction::Always => card
            .choices
            .iter()
            .find(|choice| choice.choice_id.as_deref() == Some("always")),
        InboxAnswerAction::Action => input.action_id.as_deref().and_then(|id| {
            card.choices
                .iter()
                .find(|choice| choice.choice_id.as_deref() == Some(id))
        }),
        InboxAnswerAction::Acknowledge | InboxAnswerAction::Other => None,
    }?;
    let answer = match choice.answer {
        Answer::Reject => Answer::Reject,
        _ => Answer::Approve,
    };
    Some((answer, choice.choice_id.clone()))
}

async fn acknowledge(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    let inbox = match inbox(&gateway) {
        Ok(inbox) => inbox,
        Err(response) => return response,
    };
    let person = match person(&gateway, peer, &headers) {
        Ok(person) => person,
        Err(response) => return response,
    };
    let now = crate::inbox::now_ms();
    let input = InboxAnswerInput {
        answer_id: format!("ack-{id}-{now}"),
        action: InboxAnswerAction::Acknowledge,
        answers: Vec::new(),
        action_id: None,
        comment: None,
    };
    if crate::inbox::confirmation_id(&id).is_some() {
        return answer(
            State(gateway),
            peer,
            headers,
            Path(id),
            Bytes::from(serde_json::to_vec(&input).unwrap_or_default()),
        )
        .await;
    }
    match inbox.answer(&person.username, &id, input) {
        Ok(_) => match inbox.view(&person.username, &id, now) {
            Ok(view) => Json(view).into_response(),
            Err(error) => fail(error),
        },
        Err(error) => fail(error),
    }
}

async fn read(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    let inbox = match inbox(&gateway) {
        Ok(inbox) => inbox,
        Err(response) => return response,
    };
    let person = match person(&gateway, peer, &headers) {
        Ok(person) => person,
        Err(response) => return response,
    };
    match inbox.read(&person.username, &id) {
        Ok(item) => Json(item).into_response(),
        Err(error) => fail(error),
    }
}

async fn cancel(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    let inbox = match inbox(&gateway) {
        Ok(inbox) => inbox,
        Err(response) => return response,
    };
    let person = match person(&gateway, peer, &headers) {
        Ok(person) => person,
        Err(response) => return response,
    };
    match inbox.cancel_by_viewer(&person.username, &id, None) {
        Ok(item) => Json(item).into_response(),
        Err(error) => fail(error),
    }
}

#[derive(Deserialize)]
struct SnoozeBody {
    preset: wonton_protocol::InboxSnoozePreset,
    #[serde(default)]
    timezone_offset_min: Option<i32>,
}

async fn snooze(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    body: Bytes,
) -> Response {
    let inbox = match inbox(&gateway) {
        Ok(inbox) => inbox,
        Err(response) => return response,
    };
    let person = match person(&gateway, peer, &headers) {
        Ok(person) => person,
        Err(response) => return response,
    };
    let body: SnoozeBody = match serde_json::from_slice(&body) {
        Ok(body) => body,
        Err(error) => {
            return error_response(
                StatusCode::BAD_REQUEST,
                "invalid_input",
                &format!("稍后参数不对：{error}"),
            );
        }
    };
    match inbox.snooze(&person.username, &id, body.preset, body.timezone_offset_min) {
        Ok(item) => Json(item).into_response(),
        Err(error) => fail(error),
    }
}

#[derive(Deserialize)]
struct PageQuery {
    token: String,
}

async fn page(State(gateway): State<Arc<Gateway>>, Query(query): Query<PageQuery>) -> Response {
    let inbox = match inbox(&gateway) {
        Ok(inbox) => inbox,
        Err(response) => return response,
    };
    let Some(id) = inbox
        .page_tokens()
        .resolve(&query.token, crate::inbox::now_ms())
    else {
        return error_response(StatusCode::NOT_FOUND, "not_found", "页面令牌过期了");
    };
    match inbox.page_bytes(&id) {
        Ok(Some((bytes, csp))) => Response::builder()
            .status(StatusCode::OK)
            .header("content-type", "text/html; charset=utf-8")
            .header("content-security-policy", csp)
            .header("x-content-type-options", "nosniff")
            .body(axum::body::Body::from(bytes))
            .unwrap_or_else(|_| {
                error_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "internal_error",
                    "页面没发出去",
                )
            }),
        Ok(None) => error_response(StatusCode::NOT_FOUND, "not_found", "没有页面"),
        Err(error) => fail(error),
    }
}

async fn worker_event(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let inbox = match inbox(&gateway) {
        Ok(inbox) => inbox,
        Err(response) => return response,
    };
    let person = match person(&gateway, peer, &headers) {
        Ok(person) => person,
        Err(response) => return response,
    };
    let event: InboxWorkerEvent = match serde_json::from_slice(&body) {
        Ok(event) => event,
        Err(error) => {
            return error_response(
                StatusCode::BAD_REQUEST,
                "invalid_input",
                &format!("worker 事件不对：{error}"),
            );
        }
    };
    if event.worker_id.trim().is_empty() {
        return error_response(
            StatusCode::BAD_REQUEST,
            "invalid_input",
            "worker_id 不能为空",
        );
    }
    let delivery = match worker_create(&gateway, &inbox, &person, &event).await {
        Ok(item) => item,
        Err(error) => return fail(error),
    };
    Json(delivery).into_response()
}

#[allow(clippy::too_many_lines)]
async fn worker_create(
    gateway: &Gateway,
    inbox: &Arc<Inbox>,
    person: &Person,
    event: &InboxWorkerEvent,
) -> Result<InboxItem, InboxError> {
    let (kind, title, body, payload) = match event.event {
        InboxWorkerEventKind::Needs | InboxWorkerEventKind::Stuck => (
            InboxType::Confirm,
            event.title.clone().unwrap_or_else(|| {
                format!(
                    "{} 等你批准",
                    event
                        .name
                        .clone()
                        .unwrap_or_else(|| event.worker_id.clone())
                )
            }),
            event.body.clone().unwrap_or_default(),
            InboxPayload::Confirm(ConfirmPayload {
                title: event.title.clone().unwrap_or_else(|| "需要你拍板".into()),
                body: event.body.clone().unwrap_or_default(),
                detail: None,
                risk: event.risk.unwrap_or(InboxRisk::High),
                choices: vec![
                    wonton_protocol::ConfirmChoice {
                        action: ConfirmAction::Approve,
                        label: "允许".into(),
                    },
                    wonton_protocol::ConfirmChoice {
                        action: ConfirmAction::Deny,
                        label: "拒绝".into(),
                    },
                ],
            }),
        ),
        InboxWorkerEventKind::Done if event.choices.is_empty() => (
            InboxType::Notice,
            event.title.clone().unwrap_or_else(|| "做完了".into()),
            event.body.clone().unwrap_or_default(),
            InboxPayload::Notice(NoticePayload {
                title: event.title.clone().unwrap_or_else(|| "做完了".into()),
                body: event.body.clone().unwrap_or_default(),
                level: InboxLevel::Info,
                link: None,
                action: None,
            }),
        ),
        InboxWorkerEventKind::Done => (
            InboxType::Recap,
            event.title.clone().unwrap_or_else(|| "做完了".into()),
            event.body.clone().unwrap_or_default(),
            InboxPayload::Recap(RecapPayload {
                summary: event.body.clone().unwrap_or_else(|| "worker 做完了".into()),
                question: None,
                actions: event.choices.iter().take(2).cloned().collect::<Vec<_>>(),
            }),
        ),
        InboxWorkerEventKind::Other => (
            InboxType::Notice,
            event.title.clone().unwrap_or_else(|| "worker 更新".into()),
            event.body.clone().unwrap_or_default(),
            InboxPayload::Notice(NoticePayload {
                title: event.title.clone().unwrap_or_else(|| "worker 更新".into()),
                body: event.body.clone().unwrap_or_default(),
                level: InboxLevel::Info,
                link: None,
                action: None,
            }),
        ),
    };
    let source = InboxSource {
        room_id: None,
        thread_id: None,
        worker_id: Some(event.worker_id.clone()),
        task: event.task.clone(),
    };
    let created = inbox.create(CreateInput {
        kind,
        sender: InboxSender {
            kind: InboxSenderKind::Worker,
            id: event.worker_id.clone(),
            display_name: event
                .name
                .clone()
                .unwrap_or_else(|| event.worker_id.clone()),
            avatar: None,
        },
        recipient: InboxRecipient::Person {
            username: person.username.clone(),
        },
        title,
        body,
        level: None,
        source: Some(source),
        blocking: matches!(
            event.event,
            InboxWorkerEventKind::Needs | InboxWorkerEventKind::Stuck
        ),
        dedupe_key: Some(format!("worker:{}:{:?}", event.worker_id, event.event)),
        payload,
        expires_in_ms: event
            .expires_at_ms
            .map(|at| at.saturating_sub(crate::inbox::now_ms())),
        page: None,
    })?;
    let _ = gateway;
    Ok(created.item)
}

async fn worker_answer(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
    Path(worker_id): Path<String>,
) -> Response {
    let inbox = match inbox(&gateway) {
        Ok(inbox) => inbox,
        Err(response) => return response,
    };
    let _person = match person(&gateway, peer, &headers) {
        Ok(person) => person,
        Err(response) => return response,
    };
    match inbox.latest_for_worker(&worker_id) {
        Some((item, delivery)) => Json(InboxWorkerAnswer {
            worker_id,
            item_id: item.id,
            status: item.status,
            answer: item.answer,
            confirm_action: None,
            delivery,
        })
        .into_response(),
        None => error_response(StatusCode::NOT_FOUND, "not_found", "没有这个 worker 的条目"),
    }
}

async fn worker_resolve(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
    Path(worker_id): Path<String>,
    body: Bytes,
) -> Response {
    let inbox = match inbox(&gateway) {
        Ok(inbox) => inbox,
        Err(response) => return response,
    };
    let _person = match person(&gateway, peer, &headers) {
        Ok(person) => person,
        Err(response) => return response,
    };
    let resolve: InboxWorkerResolve = match serde_json::from_slice(&body) {
        Ok(resolve) => resolve,
        Err(error) => {
            return error_response(
                StatusCode::BAD_REQUEST,
                "invalid_input",
                &format!("投递结果不对：{error}"),
            );
        }
    };
    match inbox.resolve_worker(&worker_id, resolve.delivery, resolve.error) {
        Ok(item) => Json(item).into_response(),
        Err(error) => fail(error),
    }
}

/// 页面 CSP 的默认策略：空名单断网。
#[allow(dead_code)]
fn empty_page_csp() -> String {
    policy(&[])
}
