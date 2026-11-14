//! 通用能力入口（ADR-0053）：按插件网络认出使用方，再按已确认的能力连接选择提供方。

use axum::extract::Request;
use axum::response::Response;
use hyper::header::{CONNECTION, HeaderValue};
use hyper::{HeaderMap, StatusCode};
use wonton_protocol::{BotRef, WONTON_BOT_HEADER};

use super::{Gateway, api, client_addr, pages, proxy};
use crate::l10n;
use crate::plugin::dependencies::CapabilityAccessError;

pub(super) async fn request(gateway: &Gateway, capability: &str, mut req: Request) -> Response {
    let langs = api::languages(req.headers());
    let format = pages::Format::of(req.headers());
    // agent_acp 不走字节管道：核心终结 /acp 的 WebSocket，逐帧转发（见 gateway::agent_acp）。
    // 能力的契约只有 /acp，别的路径没有。
    if capability == crate::plugin::capabilities::AGENT_ACP.name {
        return if req.uri().path() == "/acp" {
            super::agent_acp::request(gateway, req).await
        } else {
            api::error_response(
                StatusCode::NOT_FOUND,
                "invalid_input",
                &l10n::text(&langs, "gateway-agent-acp-path", &[]),
            )
        };
    }
    let Some(source) = client_addr(&req).and_then(|addr| gateway.plugins.source(addr.ip())) else {
        return api::error_response(
            StatusCode::FORBIDDEN,
            "unauthenticated",
            &l10n::text(&langs, "gateway-plugin-network-only", &[]),
        );
    };
    let bot = match caller_bot(gateway, &source.id, req.headers()) {
        Ok(bot) => bot,
        Err(()) => {
            return api::error_response(
                StatusCode::FORBIDDEN,
                "permission_denied",
                &l10n::text(
                    &langs,
                    "gateway-capability-permission",
                    &[
                        ("id", source.id.to_string().into()),
                        ("capability", capability.into()),
                    ],
                ),
            );
        }
    };
    let route = match gateway
        .plugins
        .capability_route(&source.id, capability, &langs, bot.as_ref())
        .await
    {
        Ok(route) => route,
        Err(CapabilityAccessError::Denied) => {
            return api::error_response(
                StatusCode::FORBIDDEN,
                "permission_denied",
                &l10n::text(
                    &langs,
                    "gateway-capability-permission",
                    &[
                        ("id", source.id.to_string().into()),
                        ("capability", capability.into()),
                    ],
                ),
            );
        }
        Err(CapabilityAccessError::Unavailable(detail)) => {
            return api::error_response(
                StatusCode::SERVICE_UNAVAILABLE,
                "dependency_stopped",
                &detail,
            );
        }
    };
    authenticate_caller(req.headers_mut(), &source.id.to_string(), bot.as_ref());
    let fwd = proxy::Forwarded {
        client: client_addr(&req),
        scheme: "http",
    };
    let name = route.provider.to_string();
    let target = proxy::Target {
        endpoint: &route.endpoint,
        name: &name,
    };
    match proxy::forward_or(&gateway.upstream_http, target, req, fwd, &langs).await {
        Ok(response) => response,
        Err(error) => error.response(&name, &langs, format),
    }
}

/// Repeated identity fields are ambiguous even when their values happen to agree.
pub(super) fn caller_bot(
    gateway: &Gateway,
    consumer: &crate::plugin::PluginId,
    headers: &HeaderMap,
) -> Result<Option<BotRef>, ()> {
    let mut values = headers.get_all(WONTON_BOT_HEADER).iter();
    let Some(value) = values.next() else {
        return Ok(None);
    };
    if values.next().is_some() {
        return Err(());
    }
    let name = value.to_str().unwrap_or_default().trim();
    let bot = gateway.plugins.capability_caller_bot(consumer, name);
    if bot.is_none()
        && let Some(audit) = &gateway.audit
    {
        audit.record(&serde_json::json!({
            "kind": "capability_bot_unrecognized",
            "plugin": consumer.as_str(),
            "bot": name,
        }));
    }
    Ok(bot)
}

fn trusted_identity(token: &str) -> bool {
    let token = token.trim();
    token.eq_ignore_ascii_case("x-wonton-plugin") || token.eq_ignore_ascii_case(WONTON_BOT_HEADER)
}

fn authenticate_caller(headers: &mut HeaderMap, caller: &str, bot: Option<&BotRef>) {
    // Connection 不能让转发层把刚写入的可信身份当成逐跳头删掉。其余逐跳头照旧由 proxy 处理。
    let hides_identity = headers
        .get_all(CONNECTION)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .any(trusted_identity);
    if hides_identity {
        let connection: Vec<_> = headers
            .get_all(CONNECTION)
            .iter()
            .filter_map(|value| value.to_str().ok())
            .map(|value| {
                value
                    .split(',')
                    .filter(|token| !trusted_identity(token))
                    .collect::<Vec<_>>()
                    .join(",")
            })
            .filter(|value| !value.trim().is_empty())
            .collect();
        headers.remove(CONNECTION);
        for value in connection {
            headers.append(
                CONNECTION,
                HeaderValue::from_str(&value).expect("原请求头合法"),
            );
        }
    }
    headers.insert(
        "x-wonton-plugin",
        HeaderValue::from_str(caller).expect("插件 ID 是合法的 HTTP 请求头"),
    );
    headers.remove(WONTON_BOT_HEADER);
    if let Some(bot) = bot {
        headers.insert(
            WONTON_BOT_HEADER,
            HeaderValue::from_str(&bot.bot).expect("Bot 名是合法的 HTTP 请求头"),
        );
    }
}
