use std::{
    future::Future,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
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
}

pub(crate) struct FakeAgent {
    _dir: TempDir,
    pub(crate) socket: PathBuf,
    requests: Arc<Mutex<Vec<Value>>>,
    responses: Arc<Mutex<Vec<Value>>>,
    commands: broadcast::Sender<Value>,
    task: JoinHandle<()>,
    handlers: Arc<Mutex<Vec<JoinHandle<()>>>>,
}

impl FakeAgent {
    pub(crate) async fn start(scenario: Scenario) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("acp.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let recorded = requests.clone();
        let responses = Arc::new(Mutex::new(Vec::new()));
        let recorded_responses = responses.clone();
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
        let task = tokio::spawn(async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    return;
                };
                let recorded = recorded.clone();
                let scenario = scenario.clone();
                let logged_in = task_logged_in.clone();
                let handle = tokio::spawn(serve_connection(
                    stream,
                    recorded,
                    recorded_responses.clone(),
                    connection_commands.clone(),
                    scenario,
                    logged_in,
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
            requests,
            responses,
            commands,
            task,
            handlers,
        }
    }

    pub(crate) async fn requests(&self) -> Vec<Value> {
        self.requests.lock().await.clone()
    }

    pub(crate) async fn responses(&self) -> Vec<Value> {
        self.responses.lock().await.clone()
    }

    /// 让每条连接的 Agent 发一次 `available_commands_update`。
    pub(crate) fn send_commands(&self, commands: Value) {
        let _ = self.commands.send(commands);
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
            "mcpCapabilities": {"http": {}}
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

async fn serve_connection(
    stream: UnixStream,
    recorded: Arc<Mutex<Vec<Value>>>,
    responses: Arc<Mutex<Vec<Value>>>,
    commands: broadcast::Sender<Value>,
    scenario: Scenario,
    logged_in: Arc<AtomicBool>,
) {
    let (reader, writer) = stream.into_split();
    let writer = Arc::new(Mutex::new(writer));
    let mut lines = BufReader::new(reader).lines();
    let mut command_updates = commands.subscribe();
    let (cancelled, _) = broadcast::channel(4);
    let mut prompts = 0;
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
                notify_commands(&writer, commands).await;
                continue;
            }
        };
        recorded.lock().await.push(message.clone());
        let method = message["method"].as_str().unwrap_or_default();
        if method.is_empty() {
            responses.lock().await.push(message.clone());
            continue;
        }
        match method {
            "initialize" => {
                reply(&writer, &message["id"], initialize_result(&scenario)).await;
                if matches!(scenario, Scenario::IdleElicit) {
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
                new_session(&writer, &message["id"], &scenario, &logged_in).await;
            }
            "authenticate" => {
                handle_authenticate(&writer, &message["id"], &mut lines, &scenario, &logged_in)
                    .await;
            }
            "session/load" => {
                if matches!(scenario, Scenario::RestoreMethodMissing) {
                    fail(&writer, &message["id"], "Method not found").await;
                } else {
                    reply(&writer, &message["id"], json!({})).await;
                    if matches!(scenario, Scenario::RestoreReplay) {
                        let writer = writer.clone();
                        tokio::spawn(async move {
                            tokio::time::sleep(Duration::from_millis(50)).await;
                            notify(&writer, "replay", "回放").await;
                        });
                    }
                }
            }
            "session/set_mode" if matches!(scenario, Scenario::NoModes) => {
                fail(&writer, &message["id"], "Method not found").await;
            }
            "session/set_mode" => {
                reply(&writer, &message["id"], json!({})).await;
            }
            "session/prompt" => {
                prompts += 1;
                handle_prompt(&writer, &cancelled, &message, prompts, &scenario).await;
            }
            "_session/steering" => {
                reply(&writer, &message["id"], json!({"outcome": "injected"})).await;
            }
            "session/close" => {
                reply(&writer, &message["id"], json!({})).await;
            }
            "session/cancel" => {
                let _ = cancelled.send(());
            }
            _ => {}
        }
    }
}

async fn new_session(
    writer: &Arc<Mutex<OwnedWriteHalf>>,
    id: &Value,
    scenario: &Scenario,
    logged_in: &AtomicBool,
) {
    if matches!(scenario, Scenario::LoginUrl | Scenario::LoginTerminal)
        && !logged_in.load(Ordering::Relaxed)
    {
        rpc_error(writer, id, -32000, "Authentication required").await;
    } else if matches!(scenario, Scenario::NoModes) {
        reply(writer, id, json!({"sessionId": "session-1"})).await;
    } else {
        let modes =
            json!({"currentModeId": "default", "availableModes": [{"id": "plan", "name": "Plan"}]});
        reply(
            writer,
            id,
            json!({"sessionId": "session-1", "modes": modes}),
        )
        .await;
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
            notify(&writer, "late", "迟到的旧回复").await;
        });
        return;
    }
    let text = if count == 1 { "hello" } else { "new" };
    notify(writer, "message-1", text).await;
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

async fn notify(writer: &Arc<Mutex<OwnedWriteHalf>>, message_id: &str, text: &str) {
    send(
        writer,
        json!({
            "jsonrpc": "2.0",
            "method": "session/update",
            "params": {
                "sessionId": "session-1",
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
