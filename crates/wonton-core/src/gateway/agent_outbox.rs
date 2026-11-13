//! 出箱的 HTTP 口（ADR-0061 第 9、10 节）：`GET /agent/outbox` 长轮询取项，`POST` 回报。
//! 队列、落盘和重发在 [`crate::agent_outbox`]；这一块只认调用方、解请求体。

use axum::Json;
use axum::body::to_bytes;
use axum::extract::Request;
use axum::response::{IntoResponse, Response};
use hyper::{Method, StatusCode};
use wonton_protocol::{BOT_TOKEN_HEADER, BotRef, OutboxReceipt};

use super::Gateway;
use super::api::error_response;
use crate::agent_outbox::Outbox;
use crate::mcp::Caller;

/// 回报体的上限：一项就是 ID 和一个结果，再大是调用方的错。
const MAX_BODY: usize = 16 * 1024;

/// 认出取项的 Bot：有 Bot 记录，且 Bot 的 Matrix 令牌正是这个 Bot 的（ADR-0061 第 10 节，
/// 同 /agent/chains 的核验）。批准的消息是主人的私人内容，只有握着 Bot 加密设备的桥能取，
/// Agent 自己拿总线令牌读不行。
/// `token` 由调用方先从请求头取好：别把 `&Request` 拿过 await（axum 的 Body 不是 Sync，
/// 请求的 future 会跟着不是 Send）。
// 错误分支是拼好的拒绝响应（Response 本身大），不装箱。
#[allow(clippy::result_large_err)]
async fn verified_bot(
    gateway: &Gateway,
    caller: &Caller,
    token: Option<&str>,
) -> Result<BotRef, Response> {
    let Some(bot) = bot_of(caller) else {
        return Err(denied("这个插件没有 Bot 记录，出箱只管有 Bot 的前端"));
    };
    let Some(user_id) = gateway.bot_user_of_token(token.unwrap_or_default()).await else {
        return Err(denied("缺 Bot 的 Matrix 令牌，或者令牌认不出来"));
    };
    let Ok(plugin_id) = crate::plugin::PluginId::parse(&bot.plugin_id) else {
        return Err(denied("插件 ID 认不出来"));
    };
    let account = gateway.plugins.bot_account(&plugin_id, &bot.bot);
    if account.as_ref().map(|account| account.user_id.as_str()) != Some(user_id.as_str()) {
        return Err(denied("Matrix 令牌不是这个 Bot 的"));
    }
    Ok(bot)
}

/// `GET /agent/outbox`：队里有项就给一项，没有就挂到超时，超时给 `null`。
pub(super) async fn poll(
    gateway: &Gateway,
    outbox: &Outbox,
    caller: &Caller,
    req: Request,
) -> Response {
    let token = matrix_token(&req);
    let bot = match verified_bot(gateway, caller, token.as_deref()).await {
        Ok(bot) => bot,
        Err(response) => return response,
    };
    if req.method() != Method::GET {
        return error_response(StatusCode::METHOD_NOT_ALLOWED, "invalid_input", "只收 GET");
    }
    Json(outbox.next(&bot).await).into_response()
}

/// `POST /agent/outbox`：回报取走的一项。
pub(super) async fn receipt(
    gateway: &Gateway,
    outbox: &Outbox,
    caller: &Caller,
    req: Request,
) -> Response {
    let token = matrix_token(&req);
    if let Err(response) = verified_bot(gateway, caller, token.as_deref()).await {
        return response;
    }
    if req.method() != Method::POST {
        return error_response(StatusCode::METHOD_NOT_ALLOWED, "invalid_input", "只收 POST");
    }
    let body = match to_bytes(req.into_body(), MAX_BODY).await {
        Ok(body) => body,
        Err(e) => {
            return error_response(
                StatusCode::BAD_REQUEST,
                "invalid_input",
                &format!("请求体读不出来：{e}"),
            );
        }
    };
    let receipt: OutboxReceipt = match serde_json::from_slice(&body) {
        Ok(receipt) => receipt,
        Err(e) => {
            return error_response(
                StatusCode::BAD_REQUEST,
                "invalid_input",
                &format!("请求体不是回报：{e}"),
            );
        }
    };
    if !outbox.receipt(&receipt) {
        return error_response(
            StatusCode::NOT_FOUND,
            "not_found",
            &format!("没有取走中的出箱项 {}", receipt.id),
        );
    }
    StatusCode::NO_CONTENT.into_response()
}

/// 请求头里的 Bot 的 Matrix 令牌。
fn matrix_token(req: &Request) -> Option<String> {
    req.headers()
        .get(BOT_TOKEN_HEADER)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
}

fn bot_of(caller: &Caller) -> Option<BotRef> {
    caller.bot.as_ref().map(|bot| BotRef {
        plugin_id: caller.id.to_string(),
        bot: bot.bot.clone(),
    })
}

fn denied(message: &str) -> Response {
    error_response(StatusCode::FORBIDDEN, "permission_denied", message)
}
