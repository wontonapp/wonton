use std::sync::{Arc, Mutex};

use serde_json::{Value, json};
use tokio::sync::broadcast;

use super::*;
use crate::acp::test_support::*;
use crate::acp::{AcpClient, AgentNotice};
use crate::l10n::zh;

#[derive(Clone, Default)]
struct MemorySink {
    writes: Arc<Mutex<Vec<Value>>>,
    texts: Arc<Mutex<Vec<String>>>,
    failure: Option<MatrixError>,
}

impl MemorySink {
    fn failing(failure: MatrixError) -> Self {
        Self {
            failure: Some(failure),
            ..Default::default()
        }
    }

    fn writes(&self) -> Vec<Value> {
        self.writes.lock().unwrap().clone()
    }

    fn texts(&self) -> Vec<String> {
        self.texts.lock().unwrap().clone()
    }
}

impl CommandSink for MemorySink {
    async fn set_commands(&self, content: &Value) -> wonton_matrix::Result<()> {
        self.writes.lock().unwrap().push(content.clone());
        match &self.failure {
            Some(failure) => Err(failure.clone()),
            None => Ok(()),
        }
    }

    async fn send_command_text(&self, text: &str) -> wonton_matrix::Result<()> {
        self.texts.lock().unwrap().push(text.to_owned());
        Ok(())
    }
}

async fn connected() -> (FakeAgent, AcpClient, broadcast::Receiver<AgentNotice>) {
    let agent = FakeAgent::start(Scenario::Normal).await;
    let client = bounded(AcpClient::connect(&agent.socket, broker()), "initialize")
        .await
        .unwrap();
    bounded(
        client.new_session(
            "/workspace",
            "http://mcp.wonton.internal/mcp",
            "plan",
            None,
            &origin(None),
        ),
        "session/new",
    )
    .await
    .unwrap();
    let notices = client.subscribe();
    (agent, client, notices)
}

async fn deliver(
    agent: &FakeAgent,
    notices: &mut broadcast::Receiver<AgentNotice>,
    commands: &mut Commands,
    sink: &MemorySink,
    table: Value,
) {
    agent.send_commands(table);
    let notice = bounded(notices.recv(), "命令通知").await.unwrap();
    let AgentNotice::Update { session_id, update } = notice else {
        panic!("不是 session/update");
    };
    commands
        .receive(sink, Some("session-1"), &session_id, &update, zh())
        .await;
}

fn table(entries: Value) -> Value {
    entries
}

#[tokio::test]
async fn own_command_comes_first_and_an_unchanged_table_is_not_written_again() {
    let (agent, client, mut notices) = connected().await;
    let mut commands = Commands::new();
    let sink = MemorySink::default();
    deliver(
        &agent,
        &mut notices,
        &mut commands,
        &sink,
        table(json!([
            {"name": "wonton", "description": "Agent 自己的 wonton"},
            {"name": "review", "description": "审查改动", "input": {"hint": "可选要求"}},
            {"name": "compact", "description": "压缩上下文"}
        ])),
    )
    .await;

    let writes = sink.writes();
    assert_eq!(writes.len(), 1);
    let entries = writes[0]["commands"].as_array().unwrap();
    assert_eq!(entries[0]["name"], "wonton");
    assert_eq!(
        entries[0]["description"],
        text(zh(), "cmd-description", &[])
    );
    assert!(entries[0]["hint"].as_str().unwrap().contains("login"));
    assert_eq!(entries.len(), 3);
    assert_eq!(entries[1]["name"], "review");
    assert_eq!(entries[1]["hint"], "可选要求");
    assert_eq!(entries[2]["name"], "compact");
    assert!(writes[0].get("omitted").is_none());

    deliver(
        &agent,
        &mut notices,
        &mut commands,
        &sink,
        table(json!([
            {"name": "review", "description": "审查改动", "input": {"hint": "可选要求"}},
            {"name": "compact", "description": "压缩上下文"},
            {"name": "wonton", "description": "Agent 自己的 wonton"}
        ])),
    )
    .await;
    assert_eq!(sink.writes().len(), 1);

    deliver(
        &agent,
        &mut notices,
        &mut commands,
        &sink,
        table(json!([
            {"name": "review", "description": "审查改动"},
            {"name": "compact", "description": "压缩上下文"},
            {"name": "deploy", "description": "部署"}
        ])),
    )
    .await;
    assert_eq!(sink.writes().len(), 2);
    assert_eq!(
        sink.writes()[1]["commands"][1]["name"],
        "review",
        "去掉 hint 也算表变了"
    );

    drop(client);
    agent.finish().await;
}

#[tokio::test]
async fn a_late_update_from_an_old_session_is_ignored() {
    let (agent, client, mut notices) = connected().await;
    let mut commands = Commands::new();
    let sink = MemorySink::default();
    agent.send_commands(json!([{"name": "review", "description": "审查改动"}]));
    let notice = bounded(notices.recv(), "命令通知").await.unwrap();
    let AgentNotice::Update { session_id, update } = notice else {
        panic!("不是 session/update");
    };
    commands
        .receive(&sink, Some("session-2"), &session_id, &update, zh())
        .await;
    assert!(sink.writes().is_empty());
    drop(client);
    agent.finish().await;
}

#[tokio::test]
async fn disconnect_writes_back_the_table_with_only_the_own_command() {
    let (agent, client, mut notices) = connected().await;
    let mut commands = Commands::new();
    let sink = MemorySink::default();
    deliver(
        &agent,
        &mut notices,
        &mut commands,
        &sink,
        table(json!([{"name": "review", "description": "审查改动"}])),
    )
    .await;
    assert_eq!(sink.writes().len(), 1);

    let mut closed = client.closed();
    agent.finish().await;
    bounded(closed.changed(), "Agent 断开").await.unwrap();
    assert!(*closed.borrow());

    commands.reset(&sink, zh()).await;
    let writes = sink.writes();
    assert_eq!(writes.len(), 2);
    let entries = writes[1]["commands"].as_array().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["name"], "wonton");
}

#[tokio::test]
async fn a_missing_permission_is_reported_once_per_room() {
    let (agent, client, mut notices) = connected().await;
    let mut commands = Commands::new();
    let sink = MemorySink::failing(MatrixError::Unauthorized {
        errcode: "M_FORBIDDEN".into(),
        message: "没有权限".into(),
    });
    deliver(
        &agent,
        &mut notices,
        &mut commands,
        &sink,
        table(json!([{"name": "review", "description": "审查改动"}])),
    )
    .await;
    assert_eq!(sink.texts().len(), 1);

    deliver(
        &agent,
        &mut notices,
        &mut commands,
        &sink,
        table(json!([{"name": "review", "description": "改了"}])),
    )
    .await;
    assert_eq!(sink.texts().len(), 1);
    assert_eq!(sink.writes().len(), 2, "失败的表不算写成功，下一次照样重试");

    drop(client);
    agent.finish().await;
}

#[tokio::test]
async fn other_write_errors_only_go_to_the_log() {
    let (agent, client, mut notices) = connected().await;
    let mut commands = Commands::new();
    let sink = MemorySink::failing(MatrixError::Upstream("服务器出错".into()));
    deliver(
        &agent,
        &mut notices,
        &mut commands,
        &sink,
        table(json!([{"name": "review", "description": "审查改动"}])),
    )
    .await;
    assert!(sink.texts().is_empty());
    drop(client);
    agent.finish().await;
}

#[test]
fn help_covers_the_subcommands_other_modules_handle() {
    let help = help(zh());
    for subcommand in SUBCOMMANDS {
        assert!(help.contains(&format!("/wonton {subcommand}")));
    }
    for hidden in ["interrupt", "queue", "steer"] {
        assert!(!help.contains(hidden), "{hidden} 不进帮助");
    }
    assert!(is_help_request("/wonton"));
    assert!(is_help_request("  /wonton  "));
    assert!(is_help_request("/wonton nope"));
    assert!(!is_help_request("/wonton login"));
    assert!(!is_help_request("/wonton login cancel"));
    assert!(!is_help_request("/wonton interrupt 继续"));
    assert!(!is_help_request("/wonton interrupt"));
    assert!(!is_help_request("/wonton end"));
    assert!(!is_help_request("/review"));
    assert!(!is_help_request("/wontonfoo"));
}
