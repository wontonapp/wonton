//! 连接的身份（ADR-0061 第 3、10 节，ADR-0053 的“使用方的 Bot 也是路由和确认的一维”）：
//! 发起 Bot 从使用方自报的 `X-Wonton-Bot` 读（核心核对它属于使用方插件）；说话的人从
//! `X-Wonton-On-Behalf-Of` 采纳，采纳只在他是发起 Bot 听的人时成立；目标 Bot 由核心定。

use axum::http::HeaderMap;
use axum::response::Response;
use hyper::StatusCode;
use wonton_l10n::Arg;
use wonton_protocol::{BotRef, WONTON_BOT_HEADER, WONTON_ON_BEHALF_OF_HEADER};

use super::super::{Gateway, api};
use crate::l10n;
use crate::plugin::PluginId;

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
    let Some(name) = headers
        .get(WONTON_BOT_HEADER)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|name| !name.is_empty())
    else {
        return Err(refused(
            langs,
            "gateway-agent-acp-initiator-bot-required",
            &[],
        ));
    };
    if !gateway
        .plugins
        .bot_names(consumer)
        .iter()
        .any(|bot| bot == name)
    {
        // ADR-0053：认不出的 Bot 记审计。这里随后按身份拒，行为和“当作没带”一样到不了提供方。
        if let Some(audit) = &gateway.audit {
            audit.record(&serde_json::json!({
                "kind": "agent_acp_bot_unrecognized",
                "plugin": consumer.as_str(),
                "bot": name,
            }));
        }
        return Err(refused(
            langs,
            "gateway-agent-acp-initiator-bot-unrecognized",
            &[("bot", name.into())],
        ));
    }
    Ok(BotRef {
        plugin_id: consumer.to_string(),
        bot: name.to_owned(),
    })
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

/// 目标 Bot：提供方插件（发起 Bot 的绑定）指向的那一个。提供方 Bot 绑定记在 connections.json
/// 的选择记录里（ADR-0053），绑定的核心实现在插件底座线上，接在这里；没有绑定的时候按
/// ADR-0061 第 4 节：提供方恰有一个 Bot 就用它，零个或多个都是“未就绪”，不假选。
#[expect(clippy::result_large_err)]
pub(super) fn target_bot(
    gateway: &Gateway,
    provider: &PluginId,
    langs: &crate::plugin::languages::Languages,
) -> Result<BotRef, Response> {
    let names = gateway.plugins.bot_names(provider);
    match names.as_slice() {
        [name] => Ok(BotRef {
            plugin_id: provider.to_string(),
            bot: name.clone(),
        }),
        [] => Err(unavailable(
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
