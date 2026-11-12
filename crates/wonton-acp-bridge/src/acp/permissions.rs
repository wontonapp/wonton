//! Agent 的 `session/request_permission`：合并工具调用增量、交给接管方或直接问确认卡片，
//! 以及取消（`$/cancel_request`、会话 close/cancel）和断开时的清理。
//!
//! 接管方是桥接层注册的会话级通道（ADR-0058 的排队卡片要查文件冲突），没有接管方时按
//! ADR-0035 直接请求核心的确认卡片。桥接层消失时只能回 `cancelled`：请求已经交出去，
//! 拿不回来重问。

use std::sync::Arc;

use serde_json::{Value, json};
use tokio::{io::AsyncWriteExt, sync::oneshot};

use super::{
    AgentNotice, Inner, PermissionAsk, id_key, lang_for, merge_json, parse_pending,
    permission_detail, send_result,
};
use crate::permission::{PendingPermission, PermissionResult};

pub(super) struct PermissionWaiter {
    pub(super) session_id: String,
    pub(super) cancel: oneshot::Sender<()>,
}

pub(super) async fn handle_request(inner: Arc<Inner>, id: Value, message: Value) {
    let session_id = message["params"]["sessionId"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    let mut params = message["params"].clone();
    merge_tool_call(&inner, &session_id, &mut params).await;
    let lang = lang_for(&inner, &session_id).await;
    let origin = inner.origins.lock().await.get(&session_id).cloned();
    let pending = match parse_pending(&params, lang, origin) {
        Ok(pending) => pending,
        Err(error) => {
            send_result(&inner, &id, json!({"outcome": {"outcome": "cancelled"}})).await;
            let _ = inner.notices.send(AgentNotice::PermissionFailed {
                session_id,
                detail: permission_detail(&error),
            });
            return;
        }
    };
    let key = id_key(&id);
    let (cancel_tx, cancel_rx) = oneshot::channel();
    inner.permission_waiters.lock().await.insert(
        key.clone(),
        PermissionWaiter {
            session_id: session_id.clone(),
            cancel: cancel_tx,
        },
    );
    let result = tokio::select! {
        result = decide(&inner, &session_id, pending) => result,
        _ = cancel_rx => Ok(PermissionResult::Cancelled),
    };
    inner.permission_waiters.lock().await.remove(&key);
    match result {
        Ok(PermissionResult::Selected { option_id, reason }) => {
            let mut result = json!({"outcome": {"outcome": "selected", "optionId": option_id}});
            if let Some(reason) = reason {
                result["_meta"] = json!({"reason": reason});
            }
            send_result(&inner, &id, result).await;
        }
        Ok(PermissionResult::Expired) => {
            // 过期的卡片等不到答案了（ADR-0061）：先取消会话再回 cancelled，Agent 收到
            // `session/cancel` 会撤掉这一轮。
            cancel_upstream_session(&inner, &session_id).await;
            send_result(&inner, &id, json!({"outcome": {"outcome": "cancelled"}})).await;
            let _ = inner
                .notices
                .send(AgentNotice::PermissionCancelled { session_id });
        }
        Ok(PermissionResult::Cancelled) => {
            send_result(&inner, &id, json!({"outcome": {"outcome": "cancelled"}})).await;
            let _ = inner
                .notices
                .send(AgentNotice::PermissionCancelled { session_id });
        }
        Err(error) => {
            send_result(&inner, &id, json!({"outcome": {"outcome": "cancelled"}})).await;
            let _ = inner.notices.send(AgentNotice::PermissionFailed {
                session_id,
                detail: permission_detail(&error),
            });
        }
    }
}

/// 会话有接管方时交给它（可能有冲突、要等另一轮结束才答复），没有就直接问确认卡片。
async fn decide(
    inner: &Arc<Inner>,
    session_id: &str,
    pending: PendingPermission,
) -> crate::error::Result<PermissionResult> {
    let hook = inner.permission_hooks.lock().await.get(session_id).cloned();
    if let Some(hook) = hook {
        let (reply, answer) = oneshot::channel();
        if hook
            .send(PermissionAsk {
                session_id: session_id.to_owned(),
                pending,
                reply,
            })
            .await
            .is_ok()
            && let Ok(result) = answer.await
        {
            return result;
        }
        return Ok(PermissionResult::Cancelled);
    }
    inner.permission_broker.ask(pending).await
}

/// 权限请求往往只带工具调用的增量，先把之前收到的同一 `toolCallId` 合进来再解析。
async fn merge_tool_call(inner: &Arc<Inner>, session_id: &str, params: &mut Value) {
    let Some(tool_call_id) = params["toolCall"]["toolCallId"].as_str() else {
        return;
    };
    let key = (session_id.to_owned(), tool_call_id.to_owned());
    let calls = inner.tool_calls.lock().await;
    let Some(previous) = calls.get(&key) else {
        return;
    };
    let mut merged = previous.clone();
    merge_json(&mut merged, &params["toolCall"]);
    params["toolCall"] = merged;
}

/// Agent 撤了请求（`$/cancel_request`）。
pub(super) async fn cancel_request(inner: &Arc<Inner>, request_id: &Value) {
    let key = id_key(request_id);
    if let Some(waiter) = inner.permission_waiters.lock().await.remove(&key) {
        let _ = waiter.cancel.send(());
    }
}

/// 会话被取消或关闭：它的权限请求都按取消答复。
pub(super) async fn cancel_session(inner: &Arc<Inner>, session_id: &str) {
    let mut waiters = inner.permission_waiters.lock().await;
    let keys = waiters
        .iter()
        .filter(|(_, waiter)| waiter.session_id == session_id)
        .map(|(key, _)| key.clone())
        .collect::<Vec<_>>();
    for key in keys {
        if let Some(waiter) = waiters.remove(&key) {
            let _ = waiter.cancel.send(());
        }
    }
}

/// 照 `Acp::cancel` 的同一条路取消会话：撤掉同会话待答的权限请求、记下取消标记、给
/// Agent 发 `session/cancel`。卡片过期等不到答案时先走这里再回 `cancelled`（ADR-0061）。
async fn cancel_upstream_session(inner: &Arc<Inner>, session_id: &str) {
    cancel_session(inner, session_id).await;
    inner
        .cancelled_sessions
        .lock()
        .await
        .insert(session_id.to_owned());
    let bytes = serde_json::to_vec(&json!({
        "jsonrpc": "2.0",
        "method": "session/cancel",
        "params": {"sessionId": session_id}
    }));
    if let Ok(bytes) = bytes {
        let mut writer = inner.writer.lock().await;
        let _ = writer.write_all(&bytes).await;
        let _ = writer.write_all(b"\n").await;
    }
}
