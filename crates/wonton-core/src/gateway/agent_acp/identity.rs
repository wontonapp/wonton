//! 连接的身份（ADR-0061 第 3、10 节，ADR-0053 的“使用方的 Bot 也是路由和确认的一维”）：
//! 发起 Bot 从使用方自报的 `X-Wonton-Bot` 读（核心核对它属于使用方插件）；说话的人从
//! `X-Wonton-On-Behalf-Of` 采纳，采纳只在他是发起 Bot 听的人时成立；目标 Bot 由核心定。

use axum::http::HeaderMap;
use axum::response::Response;
use hyper::StatusCode;
use wonton_l10n::Arg;
use wonton_protocol::{BotRef, WONTON_ON_BEHALF_OF_HEADER};

use super::super::{Gateway, api};
use crate::l10n;
use crate::plugin::PluginId;
use crate::plugin::dependencies::CapabilityAccessError;

/// 使用方自报的发起 Bot：路由键（ADR-0053），也是采纳说话人时“编排器的 Bot”。
/// agent_acp 的连接必须代表一个人，采纳的名单跟着发起 Bot 走，所以这里没有就建不了连接；
/// 这一点和 ADR-0053“认不出的 Bot 走插件一维”不冲突——路由按插件一维走，连接在身份这一步拒。
#[expect(clippy::result_large_err)]
pub(super) fn initiator_bot(
    gateway: &Gateway,
    consumer: &PluginId,
    headers: &HeaderMap,
    langs: &crate::plugin::languages::Languages,
) -> Result<BotRef, Response> {
    match super::super::capabilities::caller_bot(gateway, consumer, headers) {
        Ok(Some(bot)) => Ok(bot),
        Ok(None) | Err(CapabilityAccessError::Denied) => Err(refused(
            langs,
            "gateway-agent-acp-initiator-bot-required",
            &[],
        )),
        Err(CapabilityAccessError::Unavailable(detail)) => Err(api::error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "dependency_stopped",
            &detail,
        )),
    }
}

/// 链代表的人：`X-Wonton-On-Behalf-Of` 自报的说话人，只在他是发起 Bot 听的人时采纳
/// （ADR-0061 第 10 节）。没有自报、或不是听的人，都是 403——不许 Bot 以自己的名义调别的 Bot。
#[expect(clippy::result_large_err)]
pub(super) fn chain_user(
    gateway: &Gateway,
    initiator: &BotRef,
    headers: &HeaderMap,
    langs: &crate::plugin::languages::Languages,
) -> Result<String, Response> {
    let Some(claimed) = headers
        .get(WONTON_ON_BEHALF_OF_HEADER)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|claimed| !claimed.is_empty())
    else {
        return Err(refused(langs, "gateway-agent-acp-on-behalf-of", &[]));
    };
    if !gateway.bot_listens_to(initiator, claimed) {
        return Err(refused(
            langs,
            "gateway-agent-acp-adopter-not-listened",
            &[("bot", initiator.bot.as_str().into())],
        ));
    }
    Ok(claimed.to_owned())
}

/// 显式绑定必须仍然存在；没有绑定时只接受提供方唯一的 Bot。
#[expect(clippy::result_large_err)]
pub(super) fn target_bot(
    gateway: &Gateway,
    provider: &PluginId,
    selected: Option<&BotRef>,
    langs: &crate::plugin::languages::Languages,
) -> Result<BotRef, Response> {
    if let Some(selected) = selected {
        return gateway
            .plugins
            .capability_caller_bot(provider, &selected.bot)
            .map_err(|error| {
                api::error_response(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "dependency_stopped",
                    &error.to_string(),
                )
            })?
            .filter(|bot| bot == selected)
            .ok_or_else(|| {
                unavailable(
                    langs,
                    "gateway-agent-acp-no-bot",
                    &[("provider", provider.as_str().into())],
                )
            });
    }
    let mut bots = gateway
        .plugins
        .active_capability_bots(provider)
        .map_err(|error| {
            api::error_response(
                StatusCode::SERVICE_UNAVAILABLE,
                "dependency_stopped",
                &error.to_string(),
            )
        })?;
    match bots.pop() {
        Some(bot) if bots.is_empty() => Ok(bot),
        None => Err(unavailable(
            langs,
            "gateway-agent-acp-no-bot",
            &[("provider", provider.as_str().into())],
        )),
        _ => Err(unavailable(
            langs,
            "gateway-agent-acp-multi-bot",
            &[("provider", provider.as_str().into())],
        )),
    }
}

fn refused(
    langs: &crate::plugin::languages::Languages,
    id: &'static str,
    args: &[(&str, Arg<'_>)],
) -> Response {
    api::error_response(
        StatusCode::FORBIDDEN,
        "permission_denied",
        &l10n::text(langs, id, args),
    )
}

fn unavailable(
    langs: &crate::plugin::languages::Languages,
    id: &'static str,
    args: &[(&str, Arg<'_>)],
) -> Response {
    api::error_response(
        StatusCode::SERVICE_UNAVAILABLE,
        "dependency_stopped",
        &l10n::text(langs, id, args),
    )
}
