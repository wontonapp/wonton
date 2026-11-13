//! 点名叫回的唤醒标签（ADR-0061 第 10 节，第 8 块）：链上 Bot 经 `matrix.wonton.internal`
//! 发消息，核心把返回的事件 ID 记到账本（[`Chains::bind_event`]）；之后这条消息触发谁，
//! 那个 Bot 的桥报 start 就凭标签接回这条链。挂在 [`super::send_limits`] 数跳数的同一组
//! 发送上（`PUT /rooms/{房间}/send/{m.room.message|m.room.encrypted|m.sticker}/{txn}`）。
//!
//! 自带 Matrix 的 Agent（不经网关发消息）记不上标签：它的桥报 start 时没有标签，
//! 网关按点名者的归属人开新链（`agent_chains`）。

use axum::body::Body;
use axum::response::Response;
use wonton_protocol::BotRef;

use super::Gateway;
use super::send_limits::SentMessage;

/// 发送应答的上限：一条事件 ID 的 JSON，再大不认识。
const MAX_BODY: usize = 64 * 1024;

/// 转发前认好的链：消息发出去、应答带事件 ID 后挂标签。
pub(crate) struct PendingWake {
    chain: String,
}

/// 转发前：这条发送挂回哪条链（按 Bot、房间、话题找它当时活的链）。
/// 认不出 Bot 或没有活链就返回 `None`，消息照发——不是每条 Bot 消息都在链上。
pub(crate) fn prepare(gateway: &Gateway, sent: &SentMessage) -> Option<PendingWake> {
    let chains = gateway.agent_chains.as_ref()?;
    let (plugin_id, bot, _) = gateway.plugins.bot_by_user(&sent.bot_user)?;
    let bot = BotRef {
        plugin_id: plugin_id.to_string(),
        bot,
    };
    let chain = chains
        .ledger()
        .chain_for_wake(&bot, &sent.room, sent.thread.as_deref())?;
    Some(PendingWake { chain })
}

/// 发送成功后把返回的事件 ID 挂到链上。挂不上（链刚好被做完）只记日志：
/// 消息已经发出去，不能为标签失败报错或让 Bot 重发。
pub(crate) async fn after_send(
    gateway: &Gateway,
    pending: PendingWake,
    response: Response,
) -> Response {
    let (parts, body) = response.into_parts();
    if !parts.status.is_success() {
        return Response::from_parts(parts, body);
    }
    let bytes = match axum::body::to_bytes(body, MAX_BODY).await {
        Ok(bytes) => bytes,
        Err(error) => {
            tracing::warn!(error = %error, "读发送应答认事件 ID 失败，叫回标签没挂上");
            return Response::from_parts(parts, Body::empty());
        }
    };
    let event_id = serde_json::from_slice::<serde_json::Value>(&bytes)
        .ok()
        .and_then(|body| {
            body.get("event_id")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
        });
    let Some(event_id) = event_id else {
        return Response::from_parts(parts, Body::from(bytes));
    };
    if let Some(chains) = &gateway.agent_chains
        && let Err(error) = chains.ledger().bind_event(&event_id, &pending.chain)
    {
        tracing::warn!(%error, "叫回标签没挂上");
    }
    Response::from_parts(parts, Body::from(bytes))
}
