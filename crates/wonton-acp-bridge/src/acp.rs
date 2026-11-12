use std::{
    collections::{HashMap, HashSet},
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use serde_json::{Value, json};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::{UnixStream, unix::OwnedReadHalf},
    sync::{Mutex, broadcast, oneshot, watch},
};

use crate::{
    config::MAX_FRAME_BYTES,
    error::{Error, Result},
    permission::{PermissionBroker, PermissionResult, parse_pending},
};

pub mod auth;
mod mode;
mod rpc_error;
use auth::{AuthMethod, AuthRequests, parse_auth_methods};
use rpc_error::RpcError;
#[cfg(test)]
#[path = "acp_test_support.rs"]
pub(crate) mod test_support;
#[cfg(test)]
#[path = "acp_tests.rs"]
mod tests;
const CANCEL_GRACE: Duration = Duration::from_secs(1);

#[derive(Clone, Debug)]
pub struct Capabilities {
    pub load_session: bool,
    pub resume: bool,
    pub close: bool,
    pub steering: bool,
    pub mcp_http: bool,
}

#[derive(Clone, Debug)]
pub enum AgentNotice {
    PermissionCancelled { session_id: String },
    PermissionFailed { session_id: String, message: String },
    Update { session_id: String, update: Value },
}

#[derive(Debug)]
pub struct PromptResult {
    pub stop_reason: String,
    pub text: String,
    pub permissions_cancelled: usize,
    pub permission_error: Option<String>,
    pub output_limited: bool,
}

struct PendingGuard {
    inner: Arc<Inner>,
    id: u64,
}

impl Drop for PendingGuard {
    fn drop(&mut self) {
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            return;
        };
        let inner = self.inner.clone();
        let id = self.id;
        drop(handle.spawn(async move {
            if inner.pending.lock().await.remove(&id).is_some() {
                let bytes = json!({"jsonrpc":"2.0", "method":"$/cancel_request", "params":{"requestId":id}}).to_string();
                let mut writer = inner.writer.lock().await;
                let _ = writer.write_all(bytes.as_bytes()).await;
                let _ = writer.write_all(b"\n").await;
            }
        }));
    }
}

struct Inner {
    writer: Mutex<tokio::net::unix::OwnedWriteHalf>,
    next_id: AtomicU64,
    pending: Mutex<HashMap<u64, oneshot::Sender<std::result::Result<Value, RpcError>>>>,
    notices: broadcast::Sender<AgentNotice>,
    permission_broker: PermissionBroker,
    permission_waiters: Mutex<HashMap<String, PermissionWaiter>>,
    tool_calls: Mutex<HashMap<(String, String), Value>>,
    old_message_ids: Mutex<HashMap<String, HashSet<String>>>,
    cancelled_sessions: Mutex<HashSet<String>>,
    closed: watch::Sender<bool>,
    auth: AuthRequests,
}

struct PermissionWaiter {
    session_id: String,
    cancel: oneshot::Sender<()>,
}
#[derive(Clone)]
pub struct AcpClient {
    inner: Arc<Inner>,
    capabilities: Capabilities,
    auth_methods: Vec<AuthMethod>,
}

impl AcpClient {
    pub async fn connect(path: &Path, permission_broker: PermissionBroker) -> Result<Self> {
        let stream = UnixStream::connect(path).await.map_err(|error| {
            Error::AgentNotReady(format!("连接 ACP socket {} 失败：{error}", path.display()))
        })?;
        let (reader, writer) = stream.into_split();
        let (notices, _) = broadcast::channel(128);
        let inner = Arc::new(Inner {
            writer: Mutex::new(writer),
            next_id: AtomicU64::new(1),
            pending: Mutex::new(HashMap::new()),
            notices,
            permission_broker,
            permission_waiters: Mutex::new(HashMap::new()),
            tool_calls: Mutex::new(HashMap::new()),
            old_message_ids: Mutex::new(HashMap::new()),
            cancelled_sessions: Mutex::new(HashSet::new()),
            closed: watch::channel(false).0,
            auth: AuthRequests::new(),
        });
        tokio::spawn(read_loop(inner.clone(), reader));
        let client = Self {
            inner,
            capabilities: Capabilities {
                load_session: false,
                resume: false,
                close: false,
                steering: false,
                mcp_http: false,
            },
            auth_methods: Vec::new(),
        };
        let result = client
            .request(
                "initialize",
                json!({
                    "protocolVersion": 1,
                    "clientCapabilities": {
                        "elicitation": {"url": {}},
                        "auth": {"terminal": path.with_extension("login.sock").exists()}
                    },
                    "clientInfo": {"name": "wonton-acp-bridge", "version": env!("CARGO_PKG_VERSION")}
                }),
            )
            .await
            .map_err(|error| Error::AgentNotReady(error.message))?;
        Ok(Self {
            capabilities: parse_capabilities(&result),
            auth_methods: parse_auth_methods(result.get("authMethods")),
            ..client
        })
    }

    pub fn capabilities(&self) -> &Capabilities {
        &self.capabilities
    }

    pub fn closed(&self) -> watch::Receiver<bool> {
        self.inner.closed.subscribe()
    }

    pub(crate) fn subscribe(&self) -> broadcast::Receiver<AgentNotice> {
        self.inner.notices.subscribe()
    }

    pub async fn new_session(&self, cwd: &str, mcp_url: &str, mode: &str) -> Result<String> {
        if !self.capabilities.mcp_http {
            return Err(Error::AgentNotReady("Agent 未声明 HTTP MCP 能力".into()));
        }
        let result = self
            .request(
                "session/new",
                json!({
                    "cwd": cwd,
                    "mcpServers": [{"type": "http", "name": "wonton", "url": mcp_url, "headers": []}]
                }),
            )
            .await
            .map_err(|error| if error.code == -32000 { Error::AuthenticationRequired } else { Error::AgentNotReady(error.message) })?;
        let session_id = result["sessionId"]
            .as_str()
            .ok_or_else(|| Error::AgentNotReady("session/new 没有返回 sessionId".into()))?
            .to_owned();
        match mode::request(&result, &session_id, mode) {
            Some((method, params)) => {
                self.request(method, params)
                    .await
                    .map_err(|error| Error::Upstream(error.message))?;
            }
            None => tracing::info!(mode, "Agent 不分工作模式，跳过切换"),
        }
        Ok(session_id)
    }

    pub async fn load_or_resume(&self, session_id: &str, cwd: &str, mcp_url: &str) -> Result<()> {
        let params = json!({
            "sessionId": session_id,
            "cwd": cwd,
            "mcpServers": [{"type": "http", "name": "wonton", "url": mcp_url, "headers": []}]
        });
        if self.capabilities.load_session {
            return self.restore_with("session/load", params).await;
        }
        if self.capabilities.resume {
            return self.restore_with("session/resume", params).await;
        }
        Err(Error::SessionUnavailable(
            "Agent 不支持恢复 ACP session".into(),
        ))
    }
    async fn restore_with(&self, method: &str, params: Value) -> Result<()> {
        let mut notices = self.subscribe();
        self.request(method, params).await.map_err(|error| {
            if error.code == -32000 {
                return Error::AuthenticationRequired;
            }
            let detail = error.message;
            let lower = detail.to_ascii_lowercase();
            let unsupported = lower.contains("method not found")
                || lower.contains("not supported")
                || lower.contains("does not support");
            if unsupported
                || lower.contains("not found")
                || lower.contains("unknown session")
                || lower.contains("session does not exist")
            {
                Error::SessionUnavailable(detail)
            } else {
                Error::Upstream(detail)
            }
        })?;
        // 回放更新可能在响应之后才到；等它们安静下来再让下一轮 prompt 进入。
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            let idle = std::time::Duration::from_millis(300);
            match tokio::time::timeout_at(
                deadline.min(tokio::time::Instant::now() + idle),
                notices.recv(),
            )
            .await
            {
                Ok(Ok(_)) | Ok(Err(broadcast::error::RecvError::Lagged(_))) => continue,
                Ok(Err(broadcast::error::RecvError::Closed)) | Err(_) => break,
            }
        }
        Ok(())
    }

    pub async fn close(&self, session_id: &str) -> Result<()> {
        if !self.capabilities.close {
            return Ok(());
        }
        self.cancel_permissions(session_id).await;
        self.request("session/close", json!({"sessionId": session_id}))
            .await
            .map_err(|error| Error::Upstream(error.message))?;
        Ok(())
    }

    pub async fn cancel(&self, session_id: &str) -> Result<()> {
        self.cancel_permissions(session_id).await;
        self.inner
            .cancelled_sessions
            .lock()
            .await
            .insert(session_id.to_owned());
        self.notify("session/cancel", json!({"sessionId": session_id}))
            .await
    }

    pub async fn steer(&self, session_id: &str, text: &str) -> Result<String> {
        if !self.capabilities.steering {
            return Err(Error::Upstream("Agent 未声明 _session/steering".into()));
        }
        let result = self
            .request(
                "_session/steering",
                json!({
                    "sessionId": session_id,
                    "prompt": [{"type": "text", "text": text}],
                    "_meta": {"steering": {"queue": "steering", "idleBehavior": "promptRequired"}}
                }),
            )
            .await
            .map_err(|error| Error::Upstream(error.message))?;
        Ok(result["outcome"].as_str().unwrap_or("failed").to_owned())
    }

    pub async fn prompt(
        &self,
        session_id: &str,
        text: &str,
        max_output: usize,
        timeout: Duration,
    ) -> Result<PromptResult> {
        let mut notices = self.subscribe();
        if self
            .inner
            .cancelled_sessions
            .lock()
            .await
            .contains(session_id)
        {
            let deadline = tokio::time::Instant::now() + CANCEL_GRACE;
            while let Ok(Ok(notice)) = tokio::time::timeout_at(deadline, notices.recv()).await {
                let _ = notice;
            }
            self.clear_cancelled(session_id).await;
        }
        let ignored = self
            .inner
            .old_message_ids
            .lock()
            .await
            .get(session_id)
            .cloned()
            .unwrap_or_default();
        tracing::debug!(session_id, bytes = text.len(), "发送 ACP prompt");
        let response = self.request(
            "session/prompt",
            json!({"sessionId": session_id, "prompt": [{"type": "text", "text": text}]}),
        );
        tokio::pin!(response);
        let timer = tokio::time::sleep(timeout);
        tokio::pin!(timer);
        let mut updates = PromptUpdates::new(ignored, max_output);
        loop {
            tokio::select! {
                result = &mut response => {
                    let result = match result {
                        Ok(result) => result,
                        Err(error) => {
                            self.clear_cancelled(session_id).await;
                            return Err(match error.code {
                                -32000 => Error::AuthenticationRequired,
                                -32098 => Error::TransportUnavailable(error.message),
                                _ => Error::Upstream(error.message),
                            });
                        }
                    };
                    tracing::debug!(result = %result, "ACP prompt 响应");
                    self.clear_cancelled(session_id).await;
                    tokio::time::sleep(Duration::from_millis(200)).await;
                    updates.drain(&mut notices, session_id);
                    self.remember_message_ids(session_id, &updates.message_ids).await;
                    return Ok(PromptResult {
                        stop_reason: result["stopReason"].as_str().unwrap_or("end_turn").to_owned(),
                        text: updates.output,
                        permissions_cancelled: updates.permissions_cancelled,
                        permission_error: updates.permission_error,
                        output_limited: updates.output_limited,
                    });
                }
                notice = notices.recv() => match notice {
                    Ok(notice) => updates.apply(session_id, notice),
                    Err(broadcast::error::RecvError::Lagged(_)) => {}
                    Err(broadcast::error::RecvError::Closed) => return Err(Error::TransportUnavailable("ACP 连接已关闭".into())),
                },
                _ = &mut timer => {
                    let _ = self.cancel(session_id).await;
                    let response_done = tokio::time::timeout(CANCEL_GRACE, &mut response).await.is_ok();
                    tokio::time::sleep(CANCEL_GRACE).await;
                    updates.drain(&mut notices, session_id);
                    self.remember_message_ids(session_id, &updates.message_ids).await;
                    if response_done {
                        self.clear_cancelled(session_id).await;
                    }
                    return Err(Error::TransportUnavailable("ACP 单轮超时，已请求取消".into()));
                }
            }
            if updates.output_limited {
                let _ = self.cancel(session_id).await;
                let response_done = tokio::time::timeout(CANCEL_GRACE, &mut response)
                    .await
                    .is_ok();
                tokio::time::sleep(CANCEL_GRACE).await;
                updates.drain(&mut notices, session_id);
                self.remember_message_ids(session_id, &updates.message_ids)
                    .await;
                if response_done {
                    self.clear_cancelled(session_id).await;
                }
                return Err(Error::TransportUnavailable(
                    "ACP 回复超过输出限制，已请求取消".into(),
                ));
            }
        }
    }
    async fn cancel_permissions(&self, session_id: &str) {
        let mut waiters = self.inner.permission_waiters.lock().await;
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
    async fn clear_cancelled(&self, session_id: &str) {
        self.inner
            .cancelled_sessions
            .lock()
            .await
            .remove(session_id);
    }

    async fn remember_message_ids(&self, session_id: &str, ids: &HashSet<String>) {
        self.inner
            .old_message_ids
            .lock()
            .await
            .entry(session_id.to_owned())
            .or_default()
            .extend(ids.iter().cloned());
    }

    async fn request(&self, method: &str, params: Value) -> std::result::Result<Value, RpcError> {
        let id = self.inner.next_id.fetch_add(1, Ordering::Relaxed);
        let (sender, receiver) = oneshot::channel();
        self.inner.pending.lock().await.insert(id, sender);
        let _guard = PendingGuard {
            inner: self.inner.clone(),
            id,
        };
        tracing::debug!(method, id, "发送 ACP 请求");
        let message = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        if let Err(error) = self.write(message).await {
            self.inner.pending.lock().await.remove(&id);
            return Err(RpcError {
                code: -32098,
                message: error.to_string(),
            });
        }
        receiver.await.map_err(|_| RpcError {
            code: -32098,
            message: "ACP 连接已关闭".into(),
        })?
    }

    async fn notify(&self, method: &str, params: Value) -> Result<()> {
        self.write(json!({"jsonrpc": "2.0", "method": method, "params": params}))
            .await
    }

    async fn write(&self, message: Value) -> Result<()> {
        let bytes = serde_json::to_vec(&message)
            .map_err(|error| Error::InvalidInput(format!("ACP 请求编码失败：{error}")))?;
        if bytes.len() > MAX_FRAME_BYTES {
            return Err(Error::InvalidInput("ACP 请求超过单帧限制".into()));
        }
        let mut writer = self.inner.writer.lock().await;
        writer.write_all(&bytes).await?;
        writer.write_all(b"\n").await?;
        Ok(())
    }
}

async fn read_loop(inner: Arc<Inner>, reader: OwnedReadHalf) {
    let mut lines = BufReader::new(reader).lines();
    loop {
        let line = match lines.next_line().await {
            Ok(Some(line)) => line,
            Ok(None) | Err(_) => break,
        };
        if line.len() > MAX_FRAME_BYTES {
            break;
        }
        let Ok(message) = serde_json::from_str::<Value>(&line) else {
            break;
        };
        if message.get("method").is_some() {
            handle_request(&inner, &message).await;
        } else if let Some(id) = message.get("id").and_then(Value::as_u64) {
            settle_response(&inner, id, &message).await;
        }
    }
    let _ = inner.closed.send(true);
    inner.auth.close().await;
    let waiters = std::mem::take(&mut *inner.permission_waiters.lock().await);
    for (_, waiter) in waiters {
        let _ = waiter.cancel.send(());
    }
    let pending = std::mem::take(&mut *inner.pending.lock().await);
    for (_, sender) in pending {
        let _ = sender.send(Err(RpcError {
            code: -32098,
            message: "ACP Agent 连接断开".into(),
        }));
    }
}

async fn settle_response(inner: &Arc<Inner>, id: u64, message: &Value) {
    tracing::debug!(id, "收到 ACP 响应");
    let sender = inner.pending.lock().await.remove(&id);
    let Some(sender) = sender else { return };
    let result = if let Some(error) = message.get("error") {
        Err(RpcError::from_wire(error))
    } else {
        Ok(message.get("result").cloned().unwrap_or(Value::Null))
    };
    let _ = sender.send(result);
}

async fn handle_request(inner: &Arc<Inner>, message: &Value) {
    let Some(method) = message["method"].as_str() else {
        return;
    };
    tracing::debug!(method, "收到 ACP 请求");
    if method == "session/update" {
        handle_update(inner, message).await;
        return;
    }
    if method == "$/cancel_request" {
        cancel_permission_request(inner, &message["params"]["requestId"]).await;
        inner.auth.cancel(&message["params"]["requestId"]).await;
        return;
    }
    let Some(id) = message.get("id").cloned() else {
        return;
    };
    match method {
        "session/request_permission" => {
            let inner = inner.clone();
            let request = message.clone();
            tokio::spawn(async move { handle_permission_request(inner, id, request).await });
        }
        "elicitation/create" => {
            tokio::spawn(auth::elicit(inner.clone(), id, message["params"].clone()));
        }
        _ => send_error(inner, &id, -32601, "方法未实现").await,
    }
}

async fn handle_update(inner: &Arc<Inner>, message: &Value) {
    let session_id = message["params"]["sessionId"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    let update = message["params"]["update"].clone();
    if matches!(
        update["sessionUpdate"].as_str(),
        Some("tool_call" | "tool_call_update")
    ) {
        remember_tool_call(inner, &session_id, &update).await;
    }
    let _ = inner
        .notices
        .send(AgentNotice::Update { session_id, update });
}

async fn remember_tool_call(inner: &Arc<Inner>, session_id: &str, update: &Value) {
    let Some(tool_call_id) = update["toolCallId"].as_str() else {
        return;
    };
    let key = (session_id.to_owned(), tool_call_id.to_owned());
    let mut calls = inner.tool_calls.lock().await;
    let entry = calls.entry(key).or_insert_with(|| json!({}));
    merge_json(entry, update);
}

fn merge_json(target: &mut Value, patch: &Value) {
    let (Some(target), Some(patch)) = (target.as_object_mut(), patch.as_object()) else {
        *target = patch.clone();
        return;
    };
    for (key, value) in patch {
        if let Some(current) = target.get_mut(key)
            && current.is_object()
            && value.is_object()
        {
            merge_json(current, value);
            continue;
        }
        target.insert(key.clone(), value.clone());
    }
}

async fn handle_permission_request(inner: Arc<Inner>, id: Value, message: Value) {
    let session_id = message["params"]["sessionId"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    let mut params = message["params"].clone();
    merge_permission_tool_call(&inner, &session_id, &mut params).await;
    let pending = match parse_pending(&params) {
        Ok(pending) => pending,
        Err(error) => {
            send_result(&inner, &id, json!({"outcome": {"outcome": "cancelled"}})).await;
            let _ = inner.notices.send(AgentNotice::PermissionFailed {
                session_id,
                message: error.to_string(),
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
        result = inner.permission_broker.ask(pending) => result,
        _ = cancel_rx => Ok(PermissionResult::Cancelled),
    };
    inner.permission_waiters.lock().await.remove(&key);
    match result {
        Ok(PermissionResult::Selected { option_id }) => {
            send_result(
                &inner,
                &id,
                json!({"outcome": {"outcome": "selected", "optionId": option_id}}),
            )
            .await;
        }
        Ok(PermissionResult::Cancelled) | Ok(PermissionResult::Expired) => {
            send_result(&inner, &id, json!({"outcome": {"outcome": "cancelled"}})).await;
            let _ = inner
                .notices
                .send(AgentNotice::PermissionCancelled { session_id });
        }
        Err(error) => {
            send_result(&inner, &id, json!({"outcome": {"outcome": "cancelled"}})).await;
            let _ = inner.notices.send(AgentNotice::PermissionFailed {
                session_id,
                message: error.to_string(),
            });
        }
    }
}

async fn merge_permission_tool_call(inner: &Arc<Inner>, session_id: &str, params: &mut Value) {
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

async fn cancel_permission_request(inner: &Arc<Inner>, request_id: &Value) {
    let key = id_key(request_id);
    if let Some(waiter) = inner.permission_waiters.lock().await.remove(&key) {
        let _ = waiter.cancel.send(());
    }
}

async fn send_result(inner: &Arc<Inner>, id: &Value, result: Value) {
    let response = json!({"jsonrpc": "2.0", "id": id, "result": result});
    let Ok(bytes) = serde_json::to_vec(&response) else {
        return;
    };
    let mut writer = inner.writer.lock().await;
    let _ = writer.write_all(&bytes).await;
    let _ = writer.write_all(b"\n").await;
}

async fn send_error(inner: &Arc<Inner>, id: &Value, code: i64, message: &str) {
    let response = json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}});
    let Ok(bytes) = serde_json::to_vec(&response) else {
        return;
    };
    let mut writer = inner.writer.lock().await;
    let _ = writer.write_all(&bytes).await;
    let _ = writer.write_all(b"\n").await;
}

fn id_key(id: &Value) -> String {
    id.to_string()
}

fn parse_capabilities(result: &Value) -> Capabilities {
    let agent = &result["agentCapabilities"];
    let sessions = &agent["sessionCapabilities"];
    Capabilities {
        load_session: agent["loadSession"].as_bool().unwrap_or(false),
        resume: sessions.get("resume").is_some(),
        close: sessions.get("close").is_some(),
        steering: result["_meta"]["steering"]["supported"]
            .as_bool()
            .unwrap_or(false),
        mcp_http: agent["mcpCapabilities"]["http"].is_object()
            || agent["mcpCapabilities"]["http"].as_bool().unwrap_or(false),
    }
}

struct PromptUpdates {
    ignored: HashSet<String>,
    message_ids: HashSet<String>,
    output: String,
    max_output: usize,
    output_limited: bool,
    permissions_cancelled: usize,
    permission_error: Option<String>,
}

impl PromptUpdates {
    fn new(ignored: HashSet<String>, max_output: usize) -> Self {
        Self {
            ignored,
            message_ids: HashSet::new(),
            output: String::new(),
            max_output,
            output_limited: false,
            permissions_cancelled: 0,
            permission_error: None,
        }
    }

    fn apply(&mut self, session_id: &str, notice: AgentNotice) {
        match notice {
            AgentNotice::PermissionCancelled { session_id: id } if id == session_id => {
                self.permissions_cancelled += 1;
            }
            AgentNotice::PermissionFailed {
                session_id: id,
                message,
            } if id == session_id => {
                self.permission_error = Some(message);
            }
            AgentNotice::Update {
                session_id: id,
                update,
            } if id == session_id => self.append(&update),
            _ => {}
        }
    }

    fn drain(&mut self, notices: &mut broadcast::Receiver<AgentNotice>, session_id: &str) {
        while let Ok(notice) = notices.try_recv() {
            self.apply(session_id, notice);
        }
    }

    fn append(&mut self, update: &Value) {
        if let Some(message_id) = update["messageId"].as_str() {
            if self.ignored.contains(message_id) {
                return;
            }
            self.message_ids.insert(message_id.to_owned());
        }
        if update["sessionUpdate"] != "agent_message_chunk" || update["content"]["type"] != "text" {
            return;
        }
        let Some(text) = update["content"]["text"].as_str() else {
            return;
        };
        if self.output.len().saturating_add(text.len()) > self.max_output {
            self.output_limited = true;
            return;
        }
        self.output.push_str(text);
    }
}
