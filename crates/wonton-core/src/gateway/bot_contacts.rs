//! Bot 的联系人接口（ADR-0045、ADR-0050），路径见 [`wonton_protocol::bot_contacts_path`]。只收经隧道的已配对设备，
//! 设备的人要管这个 Bot：私人 Bot 是归属人，共享 Bot 是主人。错误按 [`ContactError`] 分类，头和错误体同配对接口。

use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Extension, Json, Router};
use hyper::HeaderMap;
use hyper::StatusCode;
use wonton_protocol::{
    BotContact, BotContacts, BotRef, CONTACT_ERROR_HEADER, ContactError, ContactErrorBody,
    ContactProblem, ContactRequest,
};

use super::{Gateway, TunnelPeer, api, isolation};
use crate::contacts::{ContactFailure, Contacts};
use crate::plugin::languages::Languages;

impl Gateway {
    #[must_use]
    pub fn with_contacts(mut self, contacts: Option<Arc<Contacts>>) -> Self {
        self.contacts = contacts;
        self
    }

    pub(crate) fn contacts(&self) -> Option<&Arc<Contacts>> {
        self.contacts.as_ref()
    }
}

pub(super) fn routes() -> Router<Arc<Gateway>> {
    let bot = BotRef {
        plugin_id: "{id}".to_owned(),
        bot: "{bot}".to_owned(),
    };
    Router::new()
        .route(
            &wonton_protocol::bot_contacts_path(&bot),
            get(list).post(request),
        )
        .route(
            &wonton_protocol::bot_contact_path(&bot, "{user_id}"),
            delete(remove),
        )
        .route(
            &wonton_protocol::bot_contact_approve_path(&bot, "{user_id}"),
            post(approve),
        )
        .route(
            &wonton_protocol::bot_contact_reject_path(&bot, "{user_id}"),
            post(reject),
        )
}

/// 一个联系人错误的回应：状态码按类别，头 [`CONTACT_ERROR_HEADER`]，说明按 App 的语言写（ADR-0043），原因进日志。
fn error(langs: &Languages, failure: &ContactFailure) -> Response {
    let kind = failure.kind;
    tracing::info!(kind = kind.as_str(), detail = %failure.detail, "联系人接口拒绝了请求");
    let status = kind
        .http_status()
        .and_then(|status| StatusCode::from_u16(status).ok())
        .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    let mut response = (
        status,
        Json(ContactErrorBody {
            error: ContactProblem {
                kind,
                message: message(kind, langs),
            },
        }),
    )
        .into_response();
    response.headers_mut().insert(
        CONTACT_ERROR_HEADER,
        hyper::header::HeaderValue::from_static(kind.as_str()),
    );
    response
}

pub(crate) fn message(kind: ContactError, langs: &Languages) -> String {
    let id = match kind {
        ContactError::InvalidUserId => "contacts-error-invalid-user-id",
        ContactError::NotPaired => "contacts-error-not-paired",
        ContactError::AlreadyContact => "contacts-error-already-contact",
        ContactError::BotNotFound => "contacts-error-bot-not-found",
        ContactError::ContactNotFound => "contacts-error-contact-not-found",
        ContactError::NotKeeper => "contacts-error-not-keeper",
        ContactError::PeerUnreachable => "contacts-error-peer-unreachable",
        ContactError::Unauthenticated => "contacts-error-unauthenticated",
        // 客户端核心才有的类别和比这个核心新的类别不会出现在这里。
        ContactError::Upstream
        | ContactError::TransportUnavailable
        | ContactError::Protocol
        | ContactError::Other => "contacts-error-upstream",
    };
    crate::l10n::text(langs, id, &[])
}

fn fail(headers: &HeaderMap, kind: ContactError, detail: &str) -> Response {
    error(&api::languages(headers), &ContactFailure::new(kind, detail))
}

/// 设备门（ADR-0017）：经隧道的已配对设备，请求不是别的网页发的。返回设备的人的用户名。
#[allow(clippy::result_large_err)]
fn person(
    gateway: &Gateway,
    peer: Option<Extension<TunnelPeer>>,
    headers: &HeaderMap,
) -> Result<String, Response> {
    let peer = peer.map(|Extension(peer)| peer);
    let Some(person) = gateway.device_person(peer) else {
        return Err(fail(
            headers,
            ContactError::Unauthenticated,
            "不是经隧道的已配对设备",
        ));
    };
    if !isolation::not_from_another_page(headers, &gateway.core_origin()) {
        return Err(api::error_response(
            StatusCode::FORBIDDEN,
            "cross_site",
            "联系人接口不接受其他网页发来的请求",
        ));
    }
    gateway.note_languages(peer, headers);
    Ok(person.username)
}

/// 鉴权、取联系人记录，交给 `act`。
async fn handle<F, Fut>(
    gateway: Arc<Gateway>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
    act: F,
) -> Response
where
    F: FnOnce(Arc<Contacts>, String) -> Fut,
    Fut: Future<Output = Result<Vec<BotContact>, ContactFailure>>,
{
    let username = match person(&gateway, peer, &headers) {
        Ok(username) => username,
        Err(response) => return response,
    };
    let Some(contacts) = gateway.contacts().cloned() else {
        return fail(
            &headers,
            ContactError::Upstream,
            "核心没有接入 Matrix，联系人不可用",
        );
    };
    match act(contacts, username).await {
        Ok(contacts) => Json(BotContacts { contacts }).into_response(),
        Err(failure) => error(&api::languages(&headers), &failure),
    }
}

fn bot_ref(id: String, bot: String) -> BotRef {
    BotRef { plugin_id: id, bot }
}

/// `GET`
pub async fn list(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
    Path((id, bot)): Path<(String, String)>,
) -> Response {
    let bot = bot_ref(id, bot);
    handle(gateway, peer, headers, |contacts, user| async move {
        contacts.list(&bot, &user).await
    })
    .await
}

/// `POST`：请求体 [`ContactRequest`]。
pub async fn request(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
    Path((id, bot)): Path<(String, String)>,
    body: Bytes,
) -> Response {
    let request: ContactRequest = match serde_json::from_slice(&body) {
        Ok(request) => request,
        Err(e) => {
            // 鉴权在前：没配对的设备不该知道请求体对不对。
            if let Err(response) = person(&gateway, peer, &headers) {
                return response;
            }
            return fail(&headers, ContactError::InvalidUserId, &e.to_string());
        }
    };
    let bot = bot_ref(id, bot);
    handle(gateway, peer, headers, |contacts, user| async move {
        contacts.request(&bot, &user, &request).await
    })
    .await
}

/// `POST …/approve`
pub async fn approve(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
    Path((id, bot, user_id)): Path<(String, String, String)>,
) -> Response {
    let bot = bot_ref(id, bot);
    handle(gateway, peer, headers, |contacts, user| async move {
        contacts.approve(&bot, &user, &user_id).await
    })
    .await
}

/// `POST …/reject`
pub async fn reject(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
    Path((id, bot, user_id)): Path<(String, String, String)>,
) -> Response {
    let bot = bot_ref(id, bot);
    handle(gateway, peer, headers, |contacts, user| async move {
        contacts.reject(&bot, &user, &user_id).await
    })
    .await
}

/// `DELETE`
pub async fn remove(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
    Path((id, bot, user_id)): Path<(String, String, String)>,
) -> Response {
    let bot = bot_ref(id, bot);
    handle(gateway, peer, headers, |contacts, user| async move {
        contacts.remove(&bot, &user, &user_id).await
    })
    .await
}
