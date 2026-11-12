//! Agent 给 Wonton MCP 的请求头（`session/new`、`session/load` 的 `mcpServers` 配置）：
//! 来源会话（ADR-0016）和 ACP 会话 ID（ADR-0061 第 10 节）集中在这里构造，核心据此把
//! Agent 的 MCP 调用和权限请求认到当前这一轮的链上。

use serde_json::{Value, json};
use wonton_protocol::{
    ConfirmationOrigin, ORIGIN_ROOM_HEADER, ORIGIN_THREAD_HEADER, SESSION_HEADER,
};

/// Wonton MCP 的服务器配置。`session` 是 ACP 会话 ID：`session/load` 恢复时已知，带上；
/// `session/new` 的会话 ID 要等应答才有，这次带不上（协议限制：请求头随请求发出）。
pub(super) fn wonton_mcp_server(
    mcp_url: &str,
    origin: &ConfirmationOrigin,
    session: Option<&str>,
) -> Value {
    let mut headers = vec![json!({"name": ORIGIN_ROOM_HEADER, "value": origin.room_id})];
    if let Some(thread) = &origin.thread_id {
        headers.push(json!({"name": ORIGIN_THREAD_HEADER, "value": thread}));
    }
    if let Some(session) = session {
        headers.push(json!({"name": SESSION_HEADER, "value": session}));
    }
    json!({"type": "http", "name": "wonton", "url": mcp_url, "headers": headers})
}
