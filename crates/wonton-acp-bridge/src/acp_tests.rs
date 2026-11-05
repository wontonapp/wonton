use std::{future::Future, path::PathBuf, sync::Arc, time::Duration};

use serde_json::{Value, json};
use tempfile::TempDir;
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::{UnixListener, unix::OwnedWriteHalf},
    sync::{Mutex, broadcast},
    task::JoinHandle,
};

use super::*;

enum Scenario {
    Normal,
    CancelLateUpdate,
    RestoreUnsupported,
    RestoreMethodMissing,
    RestoreReplay,
}

struct FakeAgent {
    _dir: TempDir,
    socket: PathBuf,
    requests: Arc<Mutex<Vec<Value>>>,
    task: JoinHandle<()>,
}

impl FakeAgent {
    async fn start(scenario: Scenario) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("acp.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let recorded = requests.clone();
        let task = tokio::spawn(async move {
            let Ok((stream, _)) = listener.accept().await else {
                return;
            };
            let (reader, writer) = stream.into_split();
            let writer = Arc::new(Mutex::new(writer));
            let mut lines = BufReader::new(reader).lines();
            let (cancelled, _) = broadcast::channel(4);
            let mut prompts = 0;
            while let Ok(Some(line)) = lines.next_line().await {
                let Ok(message) = serde_json::from_str::<Value>(&line) else {
                    return;
                };
                recorded.lock().await.push(message.clone());
                let method = message["method"].as_str().unwrap_or_default();
                match method {
                    "initialize" => {
                        reply(&writer, &message["id"], initialize_result(&scenario)).await;
                    }
                    "session/new" => {
                        reply(&writer, &message["id"], json!({"sessionId": "session-1"})).await;
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
        });
        Self {
            _dir: dir,
            socket,
            requests,
            task,
        }
    }

    async fn requests(&self) -> Vec<Value> {
        self.requests.lock().await.clone()
    }

    async fn finish(self) {
        self.task.abort();
        let _ = self.task.await;
    }
}

fn initialize_result(scenario: &Scenario) -> Value {
    let restore = !matches!(scenario, Scenario::RestoreUnsupported);
    json!({
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
    })
}

async fn handle_prompt(
    writer: &Arc<Mutex<OwnedWriteHalf>>,
    cancelled: &broadcast::Sender<()>,
    message: &Value,
    count: usize,
    scenario: &Scenario,
) {
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

async fn bounded<T>(operation: impl Future<Output = T>, name: &str) -> T {
    tokio::time::timeout(Duration::from_secs(5), operation)
        .await
        .unwrap_or_else(|_| panic!("{name} 超时"))
}

fn broker() -> PermissionBroker {
    PermissionBroker::new("http://127.0.0.1:1/mcp").unwrap()
}

#[tokio::test]
async fn fake_agent_round_trip_sets_http_mcp_and_mode() {
    let agent = FakeAgent::start(Scenario::Normal).await;
    let client = bounded(AcpClient::connect(&agent.socket, broker()), "initialize")
        .await
        .unwrap();
    let session = bounded(
        client.new_session("/workspace", "http://mcp.wonton.internal/mcp", "plan"),
        "session/new",
    )
    .await
    .unwrap();
    assert_eq!(session, "session-1");
    let result = bounded(
        client.prompt("session-1", "hi", 1024, Duration::from_secs(2)),
        "prompt",
    )
    .await
    .unwrap();
    assert_eq!(result.text, "hello");
    assert_eq!(
        bounded(client.steer("session-1", "continue"), "steering")
            .await
            .unwrap(),
        "injected"
    );
    bounded(client.close("session-1"), "close").await.unwrap();
    let requests = agent.requests().await;
    let new_session = requests
        .iter()
        .find(|request| request["method"] == "session/new")
        .unwrap();
    assert_eq!(new_session["params"]["mcpServers"][0]["type"], "http");
    let mode = requests
        .iter()
        .find(|request| request["method"] == "session/set_mode")
        .unwrap();
    assert_eq!(mode["params"]["modeId"], "plan");
    drop(client);
    agent.finish().await;
}

#[tokio::test]
async fn timeout_cancels_and_next_prompt_discards_late_update() {
    let agent = FakeAgent::start(Scenario::CancelLateUpdate).await;
    let client = bounded(AcpClient::connect(&agent.socket, broker()), "initialize")
        .await
        .unwrap();
    let first = bounded(
        client.prompt("session-1", "long", 1024, Duration::from_millis(50)),
        "cancelled prompt",
    )
    .await;
    assert!(matches!(first, Err(Error::TransportUnavailable(_))));
    let result = bounded(
        client.prompt("session-1", "new", 1024, Duration::from_secs(2)),
        "next prompt",
    )
    .await
    .unwrap();
    assert_eq!(result.text, "new");
    drop(client);
    agent.finish().await;
}

#[tokio::test]
async fn the_client_reports_an_agent_disconnect() {
    let agent = FakeAgent::start(Scenario::Normal).await;
    let client = bounded(AcpClient::connect(&agent.socket, broker()), "initialize")
        .await
        .unwrap();
    let mut closed = client.closed();
    assert!(!*closed.borrow());
    agent.finish().await;
    assert!(bounded(closed.changed(), "closed").await.is_ok());
    assert!(*closed.borrow());
}

#[tokio::test]
async fn restored_replay_updates_do_not_leak_into_the_next_prompt() {
    let agent = FakeAgent::start(Scenario::RestoreReplay).await;
    let client = bounded(AcpClient::connect(&agent.socket, broker()), "initialize")
        .await
        .unwrap();
    bounded(
        client.load_or_resume("session-1", "/workspace", "http://mcp.wonton.internal/mcp"),
        "session/load",
    )
    .await
    .unwrap();
    let result = bounded(
        client.prompt("session-1", "next", 1024, Duration::from_secs(2)),
        "prompt",
    )
    .await
    .unwrap();
    assert_eq!(result.text, "hello");
    assert!(!result.text.contains("回放"));
    drop(client);
    agent.finish().await;
}

#[tokio::test]
async fn missing_restore_support_is_treated_as_a_lost_session() {
    for scenario in [Scenario::RestoreUnsupported, Scenario::RestoreMethodMissing] {
        let agent = FakeAgent::start(scenario).await;
        let client = bounded(AcpClient::connect(&agent.socket, broker()), "initialize")
            .await
            .unwrap();
        let result = bounded(
            client.load_or_resume("session-1", "/workspace", "http://mcp.wonton.internal/mcp"),
            "session/load",
        )
        .await;
        assert!(matches!(result, Err(Error::SessionUnavailable(_))));
        drop(client);
        agent.finish().await;
    }
}
