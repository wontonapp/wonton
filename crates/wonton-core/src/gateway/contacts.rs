//! Bot 联系人的管理 API（ADR-0045、ADR-0050）。联系人归 Bot 的归属人管（可能是家人）；共享 Bot 只有
//! 主人能管（ADR-0045“联系人”）。都要经隧道的已配对设备（ADR-0017 的设备门），插件网页发的请求不算。
//!
//! | 方法 | 路径 | 说明 |
//! | --- | --- | --- |
//! | GET | `/_wonton/api/bots/{插件}/{Bot}/contacts` | 这个 Bot 的联系人（`BotContacts`） |
//! | POST | 同上 | 请求体 [`ContactRequest`]：发联系人请求 |
//! | POST | `…/contacts/{user_id}/approve` | 同意一条进来的请求 |
//! | POST | `…/contacts/{user_id}/reject` | 拒绝一条进来的请求，这一行随即消失 |
//! | DELETE | `…/contacts/{user_id}` | 删联系人：也取消自己发出的请求、清掉“对方拒绝了”“没有这个人” |

use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use hyper::{HeaderMap, StatusCode};
use wonton_protocol::{
    BotRef, CONTACT_ERROR_HEADER, ContactError, ContactErrorBody, ContactProblem, ContactRequest,
};

use super::isolation;
use super::{Gateway, TunnelPeer, api};

use crate::contacts::Contacts;
use crate::plugin::languages::Languages;

/// 请求者的语言，错误说明按它写（ADR-0043）。
fn languages(headers: &HeaderMap) -> Languages {
    let accept = headers
        .get(hyper::header::ACCEPT_LANGUAGE)
        .and_then(|v| v.to_str().ok());
    Languages::from_header(accept)
}

/// 联系人错误的标准回应：状态码按类别（`ContactError::http_status`），头 `X-Wonton-Error`。
fn error(langs: &Languages, kind: ContactError) -> Response {
    let status = kind
        .http_status()
        .and_then(|status| StatusCode::from_u16(status).ok())
        .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    let mut response = (
        status,
        Json(ContactErrorBody {
            error: ContactProblem {
                kind,
                message: crate::l10n::text(langs, contact_text(kind), &[]),
            },
        }),
    )
        .into_response();
    response.headers_mut().insert(
        CONTACT_ERROR_HEADER,
        hyper::header::HeaderValue::from_str(kind.as_str()).expect("类别头是合法的值"),
    );
    response
}

fn contact_text(kind: ContactError) -> &'static str {
    match kind {
        ContactError::InvalidUserId => "contacts-error-invalid-user-id",
        ContactError::NotPaired => "contacts-error-not-paired",
        ContactError::AlreadyContact => "contacts-error-already-contact",
        ContactError::BotNotFound => "contacts-error-bot-not-found",
        ContactError::ContactNotFound => "contacts-error-contact-not-found",
        ContactError::NotKeeper => "contacts-error-not-keeper",
        ContactError::PeerUnreachable => "contacts-error-peer-unreachable",
        ContactError::Unauthenticated => "contacts-error-unauthenticated",
        // 客户端核心才有的类别和比这个 App 新的类别，核心不会回应。
        ContactError::Upstream
        | ContactError::TransportUnavailable
        | ContactError::Protocol
        | ContactError::Other => "contacts-error-upstream",
    }
}

/// 没接 Matrix 的核心没有联系人：如实说不可用。
fn unavailable(langs: &Languages) -> Response {
    let mut response = (
        StatusCode::SERVICE_UNAVAILABLE,
        Json(ContactErrorBody {
            error: ContactProblem {
                kind: ContactError::Other,
                message: crate::l10n::text(langs, "contacts-error-unavailable", &[]),
            },
        }),
    )
        .into_response();
    response.headers_mut().insert(
        CONTACT_ERROR_HEADER,
        hyper::header::HeaderValue::from_str(ContactError::Other.as_str())
            .expect("类别头是合法的值"),
    );
    response
}

fn contacts(gateway: &Gateway) -> Option<&Arc<Contacts>> {
    gateway.contacts.as_ref()
}

/// 归属人门（ADR-0050）：这个 Bot 的归属人（家人也算）或主人。不是经隧道的已配对设备、
/// 或来的人既不是主人也不是这个 Bot 的归属人时，回对应的错误。
fn denied(
    gateway: &Gateway,
    peer: Option<Extension<TunnelPeer>>,
    headers: &HeaderMap,
    bot: &BotRef,
) -> Option<Response> {
    let langs = languages(headers);
    let Some(person) = gateway.device_person(peer.map(|Extension(peer)| peer)) else {
        return Some(error(&langs, ContactError::Unauthenticated));
    };
    // 插件网络里来的请求没有设备身份，也进不来（ADR-0017）：插件网页是跨站来的人。
    if !isolation::not_from_another_page(headers, &gateway.core_origin()) {
        return Some(api::error_response(
            StatusCode::FORBIDDEN,
            "cross_site",
            "这个操作要从 Wonton App 里做",
        ));
    }
    let Some(bots) = gateway.contacts.as_ref().and_then(|c| c.bots()) else {
        return Some(unavailable(&langs));
    };
    let keeper = bots.keeper_username(bot);
    let is_keeper = person.is_owner() || keeper.as_deref() == Some(person.username.as_str());
    // 共享 Bot 的归属人是主人一个人（ADR-0045）。
    let is_keeper = is_keeper && (person.is_owner() || keeper.is_some());
    if !is_keeper {
        return Some(error(&langs, ContactError::NotKeeper));
    }
    None
}

/// `GET /_wonton/api/bots/{plugin}/{bot}/contacts`
pub async fn list(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
    Path((plugin_id, bot)): Path<(String, String)>,
) -> Response {
    let bot = BotRef { plugin_id, bot };
    if let Some(denied) = denied(&gateway, peer, &headers, &bot) {
        return denied;
    }
    let langs = languages(&headers);
    let Some(contacts) = contacts(&gateway) else {
        return unavailable(&langs);
    };
    if let Err(kind) = contacts.refresh_person(&bot).await {
        return error(&langs, kind);
    }
    Json(contacts.view(&bot)).into_response()
}

/// `POST /_wonton/api/bots/{plugin}/{bot}/contacts`
pub async fn request(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
    Path((plugin_id, bot)): Path<(String, String)>,
    body: Bytes,
) -> Response {
    let bot = BotRef { plugin_id, bot };
    if let Some(denied) = denied(&gateway, peer, &headers, &bot) {
        return denied;
    }
    let langs = languages(&headers);
    let request: ContactRequest = match serde_json::from_slice(&body) {
        Ok(request) => request,
        Err(e) => {
            return api::error_response(StatusCode::BAD_REQUEST, "invalid_input", &e.to_string());
        }
    };
    let Some(contacts) = contacts(&gateway) else {
        return unavailable(&langs);
    };
    match contacts.request(&bot, &request).await {
        Ok(contacts) => Json(contacts).into_response(),
        Err(kind) => error(&langs, kind),
    }
}

/// `POST /_wonton/api/bots/{plugin}/{bot}/contacts/{user_id}/approve`
pub async fn approve(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
    Path((plugin_id, bot, user_id)): Path<(String, String, String)>,
) -> Response {
    let bot = BotRef { plugin_id, bot };
    if let Some(denied) = denied(&gateway, peer, &headers, &bot) {
        return denied;
    }
    let langs = languages(&headers);
    let Some(contacts) = contacts(&gateway) else {
        return unavailable(&langs);
    };
    match contacts.approve(&bot, &user_id).await {
        Ok(contacts) => Json(contacts).into_response(),
        Err(kind) => error(&langs, kind),
    }
}

/// `POST /_wonton/api/bots/{plugin}/{bot}/contacts/{user_id}/reject`
pub async fn reject(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
    Path((plugin_id, bot, user_id)): Path<(String, String, String)>,
) -> Response {
    let bot = BotRef { plugin_id, bot };
    if let Some(denied) = denied(&gateway, peer, &headers, &bot) {
        return denied;
    }
    let langs = languages(&headers);
    let Some(contacts) = contacts(&gateway) else {
        return unavailable(&langs);
    };
    match contacts.reject(&bot, &user_id).await {
        Ok(contacts) => Json(contacts).into_response(),
        Err(kind) => error(&langs, kind),
    }
}

/// `DELETE /_wonton/api/bots/{plugin}/{bot}/contacts/{user_id}`
pub async fn delete(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
    Path((plugin_id, bot, user_id)): Path<(String, String, String)>,
) -> Response {
    let bot = BotRef { plugin_id, bot };
    if let Some(denied) = denied(&gateway, peer, &headers, &bot) {
        return denied;
    }
    let langs = languages(&headers);
    let Some(contacts) = contacts(&gateway) else {
        return unavailable(&langs);
    };
    match contacts.delete(&bot, &user_id).await {
        Ok(contacts) => Json(contacts).into_response(),
        Err(kind) => error(&langs, kind),
    }
}
