//! 逐帧解析（ADR-0061 第 10 节）：核心终结 WebSocket，每条文本帧一条 JSON-RPC 消息。
//! 帧长度上限和桥的一致（`wonton-acp-bridge` 的 `MAX_FRAME_BYTES`）；超限由 tokio-websockets 的
//! `Limits` 报错，连接按协议错误断开。

use serde_json::Value;
use wonton_protocol::ChainError;

use crate::agent_chain::ChainLedgerError;

/// 单帧上限：桥同款。提示词带截图也到不了这个量级，超了是调用方的错。
pub(super) const MAX_FRAME_BYTES: usize = 32 * 1024 * 1024;

/// JSON-RPC 服务器错误段里留给核心自用的码；类别在 `message`，[`ChainError`] 原样在 `data`。
const CHAIN_ERROR_CODE: i64 = -32000;

/// 一帧里核心要看的部分。方法没有的是响应或通知；id 没有的不发应答。
pub(super) struct RpcFrame {
    pub(super) id: Option<Value>,
    pub(super) method: Option<String>,
    /// 应答结果里的会话 ID（`session/new`、`session/load` 的回包）：登记活会话，停链时逐个取消。
    pub(super) session_id: Option<String>,
}

/// 解析一条文本帧。不是 JSON 对象的帧按协议错误处理（调用方断开）。
pub(super) fn parse(frame: &str) -> Option<RpcFrame> {
    let value: Value = serde_json::from_str(frame).ok()?;
    let object = value.as_object()?;
    Some(RpcFrame {
        id: object.get("id").cloned().filter(|id| !id.is_null()),
        method: object
            .get("method")
            .and_then(Value::as_str)
            .map(str::to_owned),
        session_id: object
            .get("result")
            .and_then(|result| result.get("sessionId"))
            .and_then(Value::as_str)
            .map(str::to_owned),
    })
}

/// 停链时发给提供方的取消通知（ACP 规范对发 `session/cancel` 一方的要求：未决请求自己回 `cancelled`）。
pub(super) fn cancel_notification(session: &str) -> String {
    serde_json::json!({
        "jsonrpc": "2.0",
        "method": "session/cancel",
        "params": {"sessionId": session},
    })
    .to_string()
}

/// 链账本的拒绝按类别回成 JSON-RPC 错误：[`ChainError`] 原样在 `data`，消费方按类别读。
pub(super) fn error_reply(id: &Value, error: &ChainLedgerError) -> String {
    let rejection = match error {
        ChainLedgerError::Rejected(rejection) => rejection.clone(),
        ChainLedgerError::UnknownChain(_) | ChainLedgerError::UnknownSession(_) => {
            ChainError::Other
        }
        ChainLedgerError::AlreadyEnded { .. }
        | ChainLedgerError::NotAtLimit { .. }
        | ChainLedgerError::SessionsLive { .. }
        | ChainLedgerError::Invalid(_)
        | ChainLedgerError::UnknownCard(_) => ChainError::Other,
        ChainLedgerError::Corrupt { .. }
        | ChainLedgerError::Io { .. }
        | ChainLedgerError::Serialize(_) => ChainError::Other,
    };
    serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": {
            "code": CHAIN_ERROR_CODE,
            "message": category(&rejection),
            "data": rejection,
        },
    })
    .to_string()
}

/// 线上类别（同 [`super::super::agent_chains::ledger_error`] 的分法）。
fn category(rejection: &ChainError) -> &'static str {
    match rejection {
        ChainError::LimitReached { .. } => "limit_reached",
        ChainError::Stopped => "stopped",
        ChainError::Busy => "busy",
        ChainError::PermissionDenied => "permission_denied",
        ChainError::InvalidInput { .. } => "invalid_input",
        ChainError::Other => "other",
    }
}
