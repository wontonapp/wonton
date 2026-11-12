//! 每 Bot 网页的请求者授权；主机身份只读运行时登记，不从标签反推。

use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, Request, State};
use axum::response::{IntoResponse, Response};
use hyper::StatusCode;
use hyper::header::HeaderValue;
use serde_json::json;
use wonton_protocol::{BotRef, BotWebFailure, BotWebView, MemberStatus};

use super::{Gateway, Person, api, client_addr, isolation, pages};
use crate::plugin::bot_web::BotWebError;
use crate::plugin::languages::Languages;

pub(super) fn failure_status(reason: BotWebFailure) -> (StatusCode, &'static str) {
    match reason {
        BotWebFailure::Unauthenticated => (StatusCode::UNAUTHORIZED, "unauthenticated"),
        BotWebFailure::NotShared => (StatusCode::FORBIDDEN, "not_shared"),
        BotWebFailure::PermissionDenied => (StatusCode::FORBIDDEN, "permission_denied"),
        BotWebFailure::NotFound => (StatusCode::NOT_FOUND, "not_found"),
        BotWebFailure::NoWebui => (StatusCode::NOT_FOUND, "no_webui"),
        BotWebFailure::NotReady => (StatusCode::SERVICE_UNAVAILABLE, "not_ready"),
        BotWebFailure::Other => (StatusCode::INTERNAL_SERVER_ERROR, "storage"),
    }
}

pub(super) fn failure(reason: BotWebFailure, langs: &Languages) -> Response {
    let message = match reason {
        BotWebFailure::Unauthenticated => crate::l10n::text(langs, "gateway-login-required", &[]),
        BotWebFailure::NotShared => {
            crate::l10n::text(langs, "gateway-plugin-not-shared", &[("id", "Bot".into())])
        }
        BotWebFailure::PermissionDenied => {
            crate::l10n::text(langs, "gateway-bot-web-permission", &[])
        }
        BotWebFailure::NotFound => crate::l10n::text(langs, "gateway-not-found", &[]),
        BotWebFailure::NoWebui => crate::l10n::text(langs, "gateway-no-webui", &[]),
        BotWebFailure::NotReady => {
            crate::l10n::text(langs, "gateway-app-not-ready", &[("id", "Bot".into())])
        }
        BotWebFailure::Other => crate::l10n::text(langs, "gateway-bot-web-storage", &[]),
    };
    failure_response(reason, message, None)
}

fn failure_response(reason: BotWebFailure, message: String, detail: Option<String>) -> Response {
    let (status, kind) = failure_status(reason);
    let mut body = serde_json::to_value(BotWebView::Unavailable { reason }).unwrap();
    body["error"] = json!({"kind": kind, "message": message});
    if let Some(detail) = detail {
        body["error"]["detail"] = detail.into();
    }
    let mut response = (status, Json(body)).into_response();
    response
        .headers_mut()
        .insert(pages::ERROR_HEADER, HeaderValue::from_static(kind));
    response
}

pub(super) fn runtime_error(error: BotWebError, langs: &Languages) -> Response {
    match error {
        BotWebError::Unavailable(reason) => failure(reason, langs),
        BotWebError::Storage(error) => storage_error(&error, langs),
    }
}

pub(super) fn storage_error(
    error: &crate::plugin::store::StoreError,
    langs: &Languages,
) -> Response {
    tracing::warn!(%error, "读取 Bot 网页记录失败");
    failure_response(
        BotWebFailure::Other,
        crate::l10n::text(langs, "gateway-bot-web-storage", &[]),
        Some(error.to_string()),
    )
}

pub(super) fn plugin_source(gateway: &Gateway, req: &Request) -> bool {
    client_addr(req)
        .and_then(|addr| gateway.plugins.source(addr.ip()))
        .is_some()
}

pub(super) fn active_person(gateway: &Gateway, person: &Person) -> bool {
    if person.is_owner() {
        person.username == gateway.owner.username()
    } else {
        gateway
            .members
            .get(&person.username)
            .is_some_and(|record| record.status != MemberStatus::Removed)
    }
}

pub(super) fn authorize(
    gateway: &Gateway,
    bot: &BotRef,
    person: Option<&Person>,
    langs: &Languages,
) -> Result<(), Box<Response>> {
    let Some(person) = person.filter(|person| active_person(gateway, person)) else {
        return Err(Box::new(failure(BotWebFailure::Unauthenticated, langs)));
    };
    if !person.is_owner() {
        let id = crate::plugin::PluginId::parse(&bot.plugin_id)
            .map_err(|_| Box::new(failure(BotWebFailure::NotFound, langs)))?;
        let shared = gateway
            .plugins
            .family_enabled_checked(&id)
            .map_err(|error| Box::new(storage_error(&error, langs)))?;
        if !shared {
            return Err(Box::new(failure(BotWebFailure::NotShared, langs)));
        }
    }
    Ok(())
}

pub(super) fn core_person(gateway: &Gateway, req: &Request) -> Option<Person> {
    if let Some(peer) = req.extensions().get().copied() {
        gateway.device_person(Some(peer))
    } else {
        gateway.core_person(None, req.headers())
    }
}

/// 对 Bot 主机在兑换、登录以及普通插件的 self_request 例外之前拒绝所有插件网络来源。
pub(super) fn source_denied(gateway: &Gateway, req: &Request) -> Option<Response> {
    plugin_source(gateway, req).then(|| {
        failure(
            BotWebFailure::PermissionDenied,
            &api::languages(req.headers()),
        )
    })
}

pub(super) async fn get(
    State(gateway): State<Arc<Gateway>>,
    Path((plugin_id, bot)): Path<(String, String)>,
    req: Request,
) -> Response {
    if let Some(response) = source_denied(&gateway, &req) {
        return response;
    }
    let identity = BotRef { plugin_id, bot };
    let langs = api::languages(req.headers());
    let person = if isolation::not_from_another_page(req.headers(), &gateway.core_origin()) {
        core_person(&gateway, &req).or_else(|| {
            api::requester(
                &gateway,
                req.extensions().get().copied().map(axum::Extension),
                req.headers(),
            )
            .ok()
        })
    } else {
        None
    };
    if let Err(response) = authorize(&gateway, &identity, person.as_ref(), &langs) {
        return *response;
    }
    match gateway.plugins.bot_web_target(&identity) {
        Ok((_, page)) => Json(BotWebView::Ready { page }).into_response(),
        Err(error) => runtime_error(error, &langs),
    }
}

/// SSO 入口先核实精确主机登记，然后才使用人的身份和签发交接码。
pub(super) fn sso_target(
    gateway: &Gateway,
    label: &str,
    req: &Request,
    person: Option<&Person>,
) -> Result<(), Box<Response>> {
    let langs = api::languages(req.headers());
    let bot = gateway
        .plugins
        .bot_web_identity(label)
        .map_err(|error| Box::new(storage_error(&error, &langs)))?;
    let Some(bot) = bot else {
        if label.starts_with(wonton_protocol::BOT_WEB_LABEL_PREFIX) {
            if let Some(response) = source_denied(gateway, req) {
                return Err(Box::new(response));
            }
            return Err(Box::new(failure(BotWebFailure::NotFound, &langs)));
        }
        return Ok(());
    };
    if let Some(response) = source_denied(gateway, req) {
        return Err(Box::new(response));
    }
    // 未登录时继续原有的登录交接；不能提前授予地址或签发会话。
    if person.is_none() {
        return Ok(());
    }
    if let Some(peer) = req.extensions().get().copied()
        && gateway.device_person(Some(peer)).as_ref() != person
    {
        return Err(Box::new(failure(BotWebFailure::Unauthenticated, &langs)));
    }
    authorize(gateway, &bot, person, &langs)?;
    gateway
        .plugins
        .bot_web_target(&bot)
        .map_err(|error| Box::new(runtime_error(error, &langs)))?;
    Ok(())
}

pub(super) fn oidc_target(
    gateway: &Gateway,
    client_id: &str,
    req: &Request,
    person: &Person,
) -> Result<(), Box<Response>> {
    let langs = api::languages(req.headers());
    let bot = gateway
        .plugins
        .bot_web_oidc_identity(client_id)
        .map_err(|error| Box::new(storage_error(&error, &langs)))?;
    if let Some(bot) = bot {
        if let Some(response) = source_denied(gateway, req) {
            return Err(Box::new(response));
        }
        authorize(gateway, &bot, Some(person), &langs)?;
        gateway
            .plugins
            .bot_web_target(&bot)
            .map_err(|error| Box::new(runtime_error(error, &langs)))?;
    } else if client_id.starts_with(wonton_protocol::BOT_WEB_LABEL_PREFIX) {
        return Err(Box::new(failure(BotWebFailure::NotFound, &langs)));
    }
    Ok(())
}
