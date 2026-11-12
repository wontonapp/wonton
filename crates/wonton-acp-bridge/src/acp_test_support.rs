use std::{
    future::Future,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
    },
    time::Duration,
};

use serde_json::{Value, json};
use tempfile::TempDir;
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::{UnixListener, unix::OwnedWriteHalf},
    sync::{Mutex, broadcast},
    task::JoinHandle,
};

use super::*;

#[path = "acp_test_support/driver.rs"]
mod driver;

#[derive(Clone)]
pub(crate) enum Scenario {
    Normal,
    CancelLateUpdate,
    RestoreUnsupported,
    RestoreMethodMissing,
    RestoreReplay,
    LoginUrl,
    LoginTerminal,
    LoginMixed,
    IdleElicit,
    PromptTransportError,
    PromptUpstreamError,
    PromptErrorData,
    /// `session/new` 应答不带 `modes` 和 `configOptions`，也不认 `session/set_mode`（dsh 0.2.2 的实测行为）。
    NoModes,
    /// `session/prompt` 不自动应答，测试用 [`FakeAgent::respond_prompt`] 控制什么时候、回什么。
    Manual,
    /// Manual 的变体：`availableModes` 里多一个 `bypassPermissions`，测桥不跟 Agent 报出的危险档位。
    DangerousModes,
    /// 同 [`Scenario::Manual`]，另在 `promptCapabilities` 里声明能收 `image` 块。
    ManualImages,
}

/// Manual 场景里挂起的一轮：应答的 id、会话、完整内容块，和写回用的连接。
pub(crate) struct PendingPrompt {
    id: Value,
    session: String,
    text: String,
    blocks: Value,
    writer: Arc<Mutex<OwnedWriteHalf>>,
}

pub(crate) struct FakeAgent {
    _dir: TempDir,
    pub(crate) socket: PathBuf,
    commands: broadcast::Sender<Value>,
    task: JoinHandle<()>,
    handlers: Arc<Mutex<Vec<JoinHandle<()>>>>,
    link: Arc<Link>,
    next_request: Arc<AtomicU64>,
}

impl FakeAgent {
    pub(crate) async fn start(scenario: Scenario) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("acp.sock");
        let listener = UnixListener::bind(&socket).unwrap();

        let (commands, _) = broadcast::channel(8);
        let connection_commands = commands.clone();
        let logged_in = Arc::new(AtomicBool::new(false));
        let login_logged_in = logged_in.clone();
        let mut login_listener = None;
        if matches!(scenario, Scenario::LoginTerminal) {
            login_listener = Some(UnixListener::bind(socket.with_extension("login.sock")).unwrap());
        }
        let task_logged_in = logged_in.clone();
        let handlers: Arc<Mutex<Vec<JoinHandle<()>>>> = Arc::new(Mutex::new(Vec::new()));
        let recorded_handlers = handlers.clone();
        let link = Arc::new(Link {
            requests: Arc::new(Mutex::new(Vec::new())),
            responses: Arc::new(Mutex::new(Vec::new())),
            pending: Arc::new(Mutex::new(Vec::new())),
            writer: Mutex::new(None),
        });
        let task_link = link.clone();
        let task = tokio::spawn(async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    return;
                };
                let scenario = scenario.clone();
                let logged_in = task_logged_in.clone();
                let handle = tokio::spawn(serve_connection(
                    stream,
                    connection_commands.clone(),
                    scenario,
                    logged_in,
                    task_link.clone(),
                ));
                recorded_handlers.lock().await.push(handle);
            }
        });
        if let Some(listener) = login_listener {
            tokio::spawn(async move {
                let Ok((stream, _)) = listener.accept().await else {
                    return;
                };
                serve_terminal_login(stream, login_logged_in).await;
            });
        }
        Self {
            _dir: dir,
            socket,
            commands,
            task,
            handlers,
            link,
            next_request: Arc::new(AtomicU64::new(100_000)),
        }
    }

    pub(crate) async fn requests(&self) -> Vec<Value> {
        self.link.requests.lock().await.clone()
    }

    pub(crate) async fn responses(&self) -> Vec<Value> {
        self.link.responses.lock().await.clone()
    }

    /// 让每条连接的 Agent 发一次 `available_commands_update`。
    pub(crate) fn send_commands(&self, commands: Value) {
        let _ = self.commands.send(commands);
    }

    /// 还有几个挂起没答的轮次。
    pub(crate) fn pending_count(&self) -> usize {
        self.link
            .pending
            .try_lock()
            .map(|pending| pending.len())
            .unwrap_or(0)
    }

    /// 这个权限请求 id 收到的应答。
    pub(crate) async fn responses_for(&self, id: &Value) -> Vec<Value> {
        self.link
            .responses
            .lock()
            .await
            .iter()
            .filter(|response| &response["id"] == id)
            .cloned()
            .collect()
    }

    /// Manual 场景挂起的各轮：会话 ID 和发给 Agent 的原文（第一个文字块）。
    pub(crate) async fn prompts(&self) -> Vec<(String, String)> {
        self.link
            .pending
            .lock()
            .await
            .iter()
            .map(|pending| (pending.session.clone(), pending.text.clone()))
            .collect()
    }

    /// Manual 场景挂起的各轮：会话 ID 和完整内容块数组（附件轮有多个块）。
    pub(crate) async fn prompt_blocks(&self) -> Vec<(String, Value)> {
        self.link
            .pending
            .lock()
            .await
            .iter()
            .map(|pending| (pending.session.clone(), pending.blocks.clone()))
            .collect()
    }

    /// 让第 `index` 个挂起的一轮回复 `text` 并结束。
    pub(crate) async fn respond_prompt(&self, index: usize, text: &str) -> String {
        let pending = {
            let mut pending = self.link.pending.lock().await;
            pending.remove(index)
        };
        notify(
            &pending.writer,
            &pending.session,
            &format!("m-{index}"),
            text,
        )
        .await;
        reply(
            &pending.writer,
            &pending.id,
            json!({"stopReason": "end_turn"}),
        )
        .await;
        pending.session
    }

    /// 发一条工具调用通知。
    pub(crate) async fn tool_call(&self, session: &str, tool_call_id: &str, path: &str) {
        self.tool_update(session, "tool_call", tool_call_id, "edit", path)
            .await;
    }

    pub(crate) async fn tool_update(
        &self,
        session: &str,
        update: &str,
        tool_call_id: &str,
        kind: &str,
        path: &str,
    ) {
        let writer = self
            .link
            .writer
            .lock()
            .await
            .clone()
            .expect("还没有 ACP 连接");
        send(
            &writer,
            json!({
                "jsonrpc": "2.0",
                "method": "session/update",
                "params": {
                    "sessionId": session,
                    "update": {
                        "sessionUpdate": update,
                        "toolCallId": tool_call_id,
                        "title": "改文件",
                        "kind": kind,
                        "locations": [{"path": path}]
                    }
                }
            }),
        )
        .await;
    }

    /// 发一条 `_wonton/task` 任务事件（编排器扩展，ADR-0059）。
    pub(crate) async fn task(
        &self,
        session: &str,
        task_id: &str,
        title: &str,
        state: &str,
        task_session: &str,
    ) {
        let writer = self
            .link
            .writer
            .lock()
            .await
            .clone()
            .expect("还没有 ACP 连接");
        send(
            &writer,
            json!({
                "jsonrpc": "2.0",
                "method": "_wonton/task",
                "params": {
                    "sessionId": session,
                    "task": {
                        "id": task_id,
                        "title": title,
                        "state": state,
                        "taskSessionId": task_session
                    }
                }
            }),
        )
        .await;
    }

    /// 发一段原始字节，不补换行（超长帧测试用：超长行后面再跟正常帧）。
    pub(crate) async fn send_raw(&self, bytes: &[u8]) {
        let writer = self
            .link
            .writer
            .lock()
            .await
            .clone()
            .expect("还没有 ACP 连接");
        let mut writer = writer.lock().await;
        writer.write_all(bytes).await.unwrap();
        writer.flush().await.unwrap();
    }

    /// Agent 发一次改文件的权限请求，返回请求 id。
    pub(crate) async fn ask_permission(&self, session: &str, path: &str) -> Value {
        let id = json!(self.next_request.fetch_add(1, Ordering::Relaxed));
        let writer = self
            .link
            .writer
            .lock()
            .await
            .clone()
            .expect("还没有 ACP 连接");
        request(
            &writer,
            &id,
            "session/request_permission",
            json!({
                "sessionId": session,
                "toolCall": {
                    "toolCallId": "call-perm",
                    "title": format!("改 {path}"),
                    "kind": "edit",
                    "locations": [{"path": path}]
                },
                "options": [
                    {"optionId": "allow_once", "kind": "allow_once", "name": "允许"},
                    {"optionId": "reject_once", "kind": "reject_once", "name": "拒绝"}
                ]
            }),
        )
        .await;
        id
    }

    pub(crate) async fn finish(self) {
        self.task.abort();
        let handlers = self.handlers.lock().await.drain(..).collect::<Vec<_>>();
        for handler in handlers {
            handler.abort();
        }
        let _ = self.task.await;
    }
}

fn initialize_result(scenario: &Scenario) -> Value {
    let restore = !matches!(scenario, Scenario::RestoreUnsupported);
    let mut result = json!({
        "agentCapabilities": {
            "loadSession": restore,
            "sessionCapabilities": if restore {
                json!({"resume": {}, "close": {}})
            } else {
                json!({"close": {}})
            },
            "mcpCapabilities": {"http": {}},
            "promptCapabilities": if matches!(scenario, Scenario::ManualImages) {
                json!({"image": true})
            } else {
                json!({})
            }
        },
        "_meta": {"steering": {"supported": true}}
    });
    match scenario {
        Scenario::LoginUrl => {
            result["authMethods"] = json!([
                {"id": "url-method", "name": "URL 登录"}
            ])
        }
        Scenario::LoginTerminal => {
            result["authMethods"] = json!([
                {"id": "term", "name": "终端登录", "type": "terminal", "args": ["--login"]}
            ])
        }
        Scenario::LoginMixed => {
            result["authMethods"] = json!([
                {"id": "good", "name": "可用方式"},
                {"id": "env-method", "name": "环境变量方式", "type": "env_var"},
                {"id": "broken"}
            ])
        }
        Scenario::IdleElicit => {
            result["authMethods"] = json!([
                {"id": "url-method", "name": "URL 登录"}
            ])
        }
        _ => {}
    }
    result
}

/// 一条连接上测试共用的东西：记录、挂起的轮次和给 Agent 写回用的连接。
pub(crate) struct Link {
    requests: Arc<Mutex<Vec<Value>>>,
    responses: Arc<Mutex<Vec<Value>>>,
    pending: Arc<Mutex<Vec<PendingPrompt>>>,
    writer: Mutex<Option<Arc<Mutex<OwnedWriteHalf>>>>,
}

/// 一条连接的运行状态。
struct Connection {
    link: Arc<Link>,
    writer: Arc<Mutex<OwnedWriteHalf>>,
    scenario: Scenario,
    logged_in: Arc<AtomicBool>,
    cancelled: broadcast::Sender<()>,
    sessions: AtomicU32,
    prompts: usize,
}

async fn serve_connection(
    stream: UnixStream,
    commands: broadcast::Sender<Value>,
    scenario: Scenario,
    logged_in: Arc<AtomicBool>,
    link: Arc<Link>,
) {
    let (reader, writer) = stream.into_split();
    let writer = Arc::new(Mutex::new(writer));
    *link.writer.lock().await = Some(writer.clone());
    let mut lines = BufReader::new(reader).lines();
    let mut command_updates = commands.subscribe();
    let (cancelled, _) = broadcast::channel(4);
    let mut connection = Connection {
        link,
        writer,
        scenario,
        logged_in,
        cancelled,
        sessions: AtomicU32::new(0),
        prompts: 0,
    };
    loop {
        let message = tokio::select! {
            line = lines.next_line() => {
                let Ok(Some(line)) = line else {
                    return;
                };
                let Ok(message) = serde_json::from_str::<Value>(&line) else {
                    return;
                };
                message
            }
            Ok(commands) = command_updates.recv() => {
                notify_commands(&connection.writer, commands).await;
                continue;
            }
        };
        connection.link.requests.lock().await.push(message.clone());
        let method = message["method"].as_str().unwrap_or_default().to_owned();
        if method.is_empty() {
            connection.link.responses.lock().await.push(message.clone());
            continue;
        }
        if !connection.handle(&method, &message, &mut lines).await {
            return;
        }
    }
}

impl Connection {
    /// 处理一条请求；返回 false 表示连接该结束了。
    async fn handle(
        &mut self,
        method: &str,
        message: &Value,
        lines: &mut tokio::io::Lines<BufReader<tokio::net::unix::OwnedReadHalf>>,
    ) -> bool {
        let writer = &self.writer;
        match method {
            "initialize" => {
                reply(writer, &message["id"], initialize_result(&self.scenario)).await;
                if matches!(self.scenario, Scenario::IdleElicit) {
                    let writer = writer.clone();
                    tokio::spawn(async move {
                        request(
                            &writer,
                            &json!(7_000),
                            "elicitation/create",
                            json!({
                                "mode": "url",
                                "url": "https://auth.example/idle",
                                "message": "空闲时的引导"
                            }),
                        )
                        .await;
                    });
                }
            }
            "session/new" => {
                // 每条连接（一次桥进程）自己编号，和真实 Agent 一样重连后从 1 再来。
                let id = json!(format!(
                    "session-{}",
                    self.sessions.fetch_add(1, Ordering::Relaxed) + 1
                ));
                new_session(writer, &message["id"], &id, &self.scenario, &self.logged_in).await;
            }
            "authenticate" => {
                handle_authenticate(
                    writer,
                    &message["id"],
                    lines,
                    &self.scenario,
                    &self.logged_in,
                )
                .await;
            }
            "session/load" => {
                if matches!(self.scenario, Scenario::RestoreMethodMissing) {
                    fail(writer, &message["id"], "Method not found").await;
                } else {
                    reply(writer, &message["id"], json!({})).await;
                    if matches!(self.scenario, Scenario::RestoreReplay) {
                        let writer = writer.clone();
                        tokio::spawn(async move {
                            tokio::time::sleep(Duration::from_millis(50)).await;
                            notify(&writer, "session-1", "replay", "回放").await;
                        });
                    }
                }
            }
            "session/set_mode" if matches!(self.scenario, Scenario::NoModes) => {
                fail(writer, &message["id"], "Method not found").await;
            }
            "session/set_mode" => {
                reply(writer, &message["id"], json!({})).await;
            }
            "session/prompt" => {
                self.prompts += 1;
                if matches!(
                    self.scenario,
                    Scenario::Manual | Scenario::DangerousModes | Scenario::ManualImages
                ) {
                    self.hold_prompt(message, writer).await;
                } else {
                    handle_prompt(
                        writer,
                        &self.cancelled,
                        message,
                        self.prompts,
                        &self.scenario,
                    )
                    .await;
                }
            }
            "_session/steering" => {
                reply(writer, &message["id"], json!({"outcome": "injected"})).await;
            }
            "session/close" => {
                reply(writer, &message["id"], json!({})).await;
            }
            "session/cancel" => {
                let _ = self.cancelled.send(());
            }
            _ => {}
        }
        true
    }

    /// Manual 系场景：把一轮挂在链路上，等测试用 [`FakeAgent::respond_prompt`] 回。
    async fn hold_prompt(&self, message: &Value, writer: &Arc<Mutex<OwnedWriteHalf>>) {
        self.link.pending.lock().await.push(PendingPrompt {
            id: message["id"].clone(),
            session: message["params"]["sessionId"]
                .as_str()
                .unwrap_or_default()
                .to_owned(),
            text: message["params"]["prompt"][0]["text"]
                .as_str()
                .unwrap_or_default()
                .to_owned(),
            blocks: message["params"]["prompt"].clone(),
            writer: writer.clone(),
        });
    }
}

async fn new_session(
    writer: &Arc<Mutex<OwnedWriteHalf>>,
    id: &Value,
    session: &Value,
    scenario: &Scenario,
    logged_in: &AtomicBool,
) {
    if matches!(scenario, Scenario::LoginUrl | Scenario::LoginTerminal)
        && !logged_in.load(Ordering::Relaxed)
    {
        rpc_error(writer, id, -32000, "Authentication required").await;
    } else if matches!(scenario, Scenario::NoModes) {
        reply(writer, id, json!({"sessionId": session})).await;
    } else {
        let mut modes = json!({
            "currentModeId": "plan",
            "availableModes": [
                {"id": "plan", "name": "Plan"},
                {"id": "workspace-write", "name": "Workspace write"},
                {"id": "agent", "name": "Agent"}
            ]
        });
        if matches!(scenario, Scenario::DangerousModes) {
            modes["availableModes"]
                .as_array_mut()
                .unwrap()
                .push(json!({"id": "bypassPermissions", "name": "Bypass"}));
        }
        reply(writer, id, json!({"sessionId": session, "modes": modes})).await;
    }
}

async fn handle_authenticate(
    writer: &Arc<Mutex<OwnedWriteHalf>>,
    id: &Value,
    lines: &mut tokio::io::Lines<BufReader<tokio::net::unix::OwnedReadHalf>>,
    scenario: &Scenario,
    logged_in: &Arc<AtomicBool>,
) {
    if !matches!(scenario, Scenario::LoginUrl) {
        rpc_error(writer, id, -32601, "Method not found").await;
        return;
    }
    let request_id = json!(9_000);
    request(
        writer,
        &request_id,
        "elicitation/create",
        json!({
            "mode": "url",
            "elicitationId": "login-1",
            "url": "https://auth.example/login?code=abc",
            "message": "请打开链接完成登录"
        }),
    )
    .await;
    let action = loop {
        let Ok(Some(line)) = bounded(lines.next_line(), "等待 elicitation 应答")
            .await
            .map_err(|_| ())
        else {
            return;
        };
        let Ok(message) = serde_json::from_str::<Value>(&line) else {
            return;
        };
        if message["id"] == request_id {
            break message["result"]["action"]
                .as_str()
                .unwrap_or("cancel")
                .to_owned();
        }
    };
    if action == "accept" {
        logged_in.store(true, Ordering::Relaxed);
        reply(writer, id, json!({})).await;
    } else {
        rpc_error(writer, id, -32000, "登录被拒绝").await;
    }
}

async fn serve_terminal_login(stream: UnixStream, logged_in: Arc<AtomicBool>) {
    let (reader, mut writer) = stream.into_split();
    let mut lines = BufReader::new(reader).lines();
    let Some(request) = lines.next_line().await.unwrap() else {
        return;
    };
    let request: Value = serde_json::from_str(&request).unwrap();
    assert_eq!(request["methodId"], "term");
    assert_eq!(request["args"][0], "--login");
    // 链接打在 stderr，验证桥也把它送进聊天。
    send_frame(&mut writer, json!({"err": "https://auth.example/device"})).await;
    send_frame(&mut writer, json!({"out": "输入授权码："})).await;
    for line in [] as [&str; 0] {
        send_frame(&mut writer, json!({"out": line})).await;
    }
    let Some(code) = lines.next_line().await.unwrap() else {
        return;
    };
    assert_eq!(code, "code-42");
    logged_in.store(true, Ordering::Relaxed);
    send_frame(&mut writer, json!({"exit": 0})).await;
}

async fn send_frame(writer: &mut tokio::net::unix::OwnedWriteHalf, frame: Value) {
    writer
        .write_all(frame.to_string().as_bytes())
        .await
        .unwrap();
    writer.write_all(b"\n").await.unwrap();
    writer.flush().await.unwrap();
}

async fn request(writer: &Arc<Mutex<OwnedWriteHalf>>, id: &Value, method: &str, params: Value) {
    send(
        writer,
        json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}),
    )
    .await;
}

async fn rpc_error(writer: &Arc<Mutex<OwnedWriteHalf>>, id: &Value, code: i64, message: &str) {
    send(
        writer,
        json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}}),
    )
    .await;
}

async fn handle_prompt(
    writer: &Arc<Mutex<OwnedWriteHalf>>,
    cancelled: &broadcast::Sender<()>,
    message: &Value,
    count: usize,
    scenario: &Scenario,
) {
    let session = message["params"]["sessionId"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    if matches!(scenario, Scenario::PromptTransportError) {
        rpc_error(writer, &message["id"], -32098, "ACP 连接已关闭").await;
        return;
    }
    if matches!(scenario, Scenario::PromptUpstreamError) {
        rpc_error(writer, &message["id"], -32603, "上游炸了").await;
        return;
    }
    if matches!(scenario, Scenario::PromptErrorData) {
        let long_detail = "细节。".repeat(800);
        let error = match count {
            1 => {
                json!({"code": -32603, "message": "Internal error", "data": {"details": "No model selected. Use /login 选择模型。"}})
            }
            2 => {
                json!({"code": -32603, "message": "Internal error", "data": "后台正在编译，请稍后再试"})
            }
            3 => {
                json!({"code": -32603, "message": "同一个原因", "data": {"message": "同一个原因"}})
            }
            4 => json!({"code": -32603, "message": "Internal error", "data": {"details": 42}}),
            _ => {
                json!({"code": -32603, "message": "Internal error", "data": {"details": long_detail}})
            }
        };
        send(
            writer,
            json!({"jsonrpc": "2.0", "id": message["id"], "error": error}),
        )
        .await;
        return;
    }
    if matches!(scenario, Scenario::CancelLateUpdate) && count == 1 {
        let mut cancellation = cancelled.subscribe();
        let writer = writer.clone();
        let request_id = message["id"].clone();
        tokio::spawn(async move {
            let _ = cancellation.recv().await;
            reply(&writer, &request_id, json!({"stopReason": "cancelled"})).await;
            tokio::time::sleep(Duration::from_millis(150)).await;
            notify(&writer, "session-1", "late", "迟到的旧回复").await;
        });
        return;
    }
    let text = if count == 1 { "hello" } else { "new" };
    notify(writer, &session, "message-1", text).await;
    reply(writer, &message["id"], json!({"stopReason": "end_turn"})).await;
}

async fn notify_commands(writer: &Arc<Mutex<OwnedWriteHalf>>, commands: Value) {
    send(
        writer,
        json!({
            "jsonrpc": "2.0",
            "method": "session/update",
            "params": {
                "sessionId": "session-1",
                "update": {
                    "sessionUpdate": "available_commands_update",
                    "availableCommands": commands
                }
            }
        }),
    )
    .await;
}

async fn notify(writer: &Arc<Mutex<OwnedWriteHalf>>, session: &str, message_id: &str, text: &str) {
    send(
        writer,
        json!({
            "jsonrpc": "2.0",
            "method": "session/update",
            "params": {
                "sessionId": session,
                "update": {
                    "sessionUpdate": "agent_message_chunk",
                    "messageId": message_id,
                    "content": {"type": "text", "text": text}
                }
            }
        }),
    )
    .await;
}

async fn reply(writer: &Arc<Mutex<OwnedWriteHalf>>, id: &Value, result: Value) {
    send(
        writer,
        json!({"jsonrpc": "2.0", "id": id, "result": result}),
    )
    .await;
}

async fn fail(writer: &Arc<Mutex<OwnedWriteHalf>>, id: &Value, message: &str) {
    send(
        writer,
        json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32601, "message": message}}),
    )
    .await;
}

async fn send(writer: &Arc<Mutex<OwnedWriteHalf>>, message: Value) {
    let bytes = serde_json::to_vec(&message).unwrap();
    let mut writer = writer.lock().await;
    writer.write_all(&bytes).await.unwrap();
    writer.write_all(b"\n").await.unwrap();
    writer.flush().await.unwrap();
}

pub(crate) async fn bounded<T>(operation: impl Future<Output = T>, name: &str) -> T {
    tokio::time::timeout(Duration::from_secs(5), operation)
        .await
        .unwrap_or_else(|_| panic!("{name} 超时"))
}

pub(crate) fn broker() -> PermissionBroker {
    PermissionBroker::new("http://127.0.0.1:1/mcp").unwrap()
}

/// 测试用的来源会话：固定房间，`thread` 是话题根事件 ID。
pub(crate) fn origin(thread: Option<&str>) -> ConfirmationOrigin {
    ConfirmationOrigin {
        room_id: "!room:wonton.internal".into(),
        thread_id: thread.map(str::to_owned),
    }
}
