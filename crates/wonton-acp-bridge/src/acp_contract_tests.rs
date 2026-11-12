//! 报文契约测试（ADR-0035）：桥自己拼的 JSON 必须能被官方 [`agent_client_protocol`]
//! 的 v1 类型原样读出来，官方类型序列化出的 JSON 也必须被桥原样读懂。哪边改了
//! 字段映射，这里的断言先红。

use std::time::Duration;

use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use agent_client_protocol::schema as acp;

use super::*;
use crate::acp::test_support::*;
use crate::l10n::zh;
use wonton_protocol::ORIGIN_ROOM_HEADER;

fn as_official<T: DeserializeOwned>(value: &Value) -> T {
    serde_json::from_value(value.clone())
        .unwrap_or_else(|error| panic!("官方类型解析失败：{error}\n{value}"))
}

async fn request_of(agent: &FakeAgent, method: &str) -> Value {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        let requests = agent.requests().await;
        if let Some(request) = requests.iter().find(|request| request["method"] == method) {
            return request.clone();
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "没有发出 {method}：{requests:?}"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

async fn response_of(agent: &FakeAgent, id: &Value) -> Value {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        let responses = agent.responses().await;
        if let Some(response) = responses.iter().find(|response| &response["id"] == id) {
            return response.clone();
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "没有对 {id} 的应答：{responses:?}"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

async fn connect_normal() -> (FakeAgent, AcpClient) {
    let agent = FakeAgent::start(Scenario::Normal).await;
    let client = bounded(AcpClient::connect(&agent.socket, broker()), "initialize")
        .await
        .unwrap();
    (agent, client)
}

// 桥发出的报文 → 官方 v1 类型

#[tokio::test]
async fn outgoing_initialize_matches_official_schema() {
    let (agent, client) = connect_normal().await;
    let request = request_of(&agent, "initialize").await;
    let params = as_official::<acp::InitializeRequest>(&request["params"]);
    assert_eq!(params.protocol_version, acp::ProtocolVersion::V1);
    let info = params.client_info.expect("clientInfo");
    assert_eq!(info.name, "wonton-acp-bridge");
    assert_eq!(info.version, env!("CARGO_PKG_VERSION"));
    // 桥报的 clientCapabilities.elicitation/auth 是 0.13 之后才进官方 schema 的键：
    // 0.13 类型按未知键忽略，兼容但这里断言不了（1.17 起才有对应字段）。
    drop(client);
    agent.finish().await;
}

#[tokio::test]
async fn outgoing_session_new_and_set_mode_match_official_schema() {
    let (agent, client) = connect_normal().await;
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
    let params =
        as_official::<acp::NewSessionRequest>(&request_of(&agent, "session/new").await["params"]);
    assert_eq!(params.cwd, std::path::Path::new("/workspace"));
    let acp::McpServer::Http(server) = &params.mcp_servers[0] else {
        panic!("MCP 服务器应是 http：{:?}", params.mcp_servers[0]);
    };
    assert_eq!(server.name, "wonton");
    assert_eq!(server.url, "http://mcp.wonton.internal/mcp");
    // 来源会话进 MCP 请求头（ADR-0016）：测试来源只有房间，没有话题。
    // HttpHeader 是 non_exhaustive，经官方反序列化构造对照值。
    let headers: Vec<acp::HttpHeader> =
        as_official(&json!([{"name": ORIGIN_ROOM_HEADER, "value": "!room:wonton.internal"}]));
    assert_eq!(server.headers, headers);
    let mode = as_official::<acp::SetSessionModeRequest>(
        &request_of(&agent, "session/set_mode").await["params"],
    );
    assert_eq!(mode.session_id.0.as_ref(), "session-1");
    assert_eq!(mode.mode_id.0.as_ref(), "plan");
    drop(client);
    agent.finish().await;
}

#[tokio::test]
async fn outgoing_prompt_cancel_and_close_match_official_schema() {
    let (agent, client) = connect_normal().await;
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
    bounded(
        client.prompt(
            "session-1",
            vec![json!({"type": "text", "text": "hi"})],
            1024,
            Duration::from_secs(2),
            zh(),
        ),
        "prompt",
    )
    .await
    .unwrap();
    let params =
        as_official::<acp::PromptRequest>(&request_of(&agent, "session/prompt").await["params"]);
    assert_eq!(params.session_id.0.as_ref(), "session-1");
    let [acp::ContentBlock::Text(text)] = &params.prompt[..] else {
        panic!("prompt 应是一条 text：{:?}", params.prompt);
    };
    assert_eq!(text.text, "hi");
    bounded(client.cancel("session-1"), "cancel").await.unwrap();
    let cancel = as_official::<acp::CancelNotification>(
        &request_of(&agent, "session/cancel").await["params"],
    );
    assert_eq!(cancel.session_id.0.as_ref(), "session-1");
    bounded(client.close("session-1"), "close").await.unwrap();
    let close = as_official::<acp::CloseSessionRequest>(
        &request_of(&agent, "session/close").await["params"],
    );
    assert_eq!(close.session_id.0.as_ref(), "session-1");
    drop(client);
    agent.finish().await;
}

#[tokio::test]
async fn outgoing_session_load_matches_official_schema() {
    let (agent, client) = connect_normal().await;
    bounded(
        client.load_or_resume(
            "s-9",
            "/workspace",
            "http://mcp.wonton.internal/mcp",
            &origin(None),
            "plan",
        ),
        "session/load",
    )
    .await
    .unwrap();
    let params =
        as_official::<acp::LoadSessionRequest>(&request_of(&agent, "session/load").await["params"]);
    assert_eq!(params.session_id.0.as_ref(), "s-9");
    assert_eq!(params.cwd, std::path::Path::new("/workspace"));
    assert_eq!(params.mcp_servers.len(), 1);
    // 只报 resume 能力的 Agent（dsh 0.2.2 的写法）收到同样的参数，方法名换成 session/resume。
    let resume = as_official::<acp::ResumeSessionRequest>(
        &request_of(&agent, "session/load").await["params"],
    );
    assert_eq!(resume.session_id.0.as_ref(), "s-9");
    drop(client);
    agent.finish().await;
}

// 官方 v1 类型生成的报文 → 桥的解析

#[tokio::test]
async fn official_initialize_response_is_understood() {
    let mut response = acp::InitializeResponse::new(acp::ProtocolVersion::V1);
    response.agent_capabilities.load_session = true;
    response.agent_capabilities.mcp_capabilities.http = true;
    response.agent_capabilities.session_capabilities.close =
        Some(acp::SessionCloseCapabilities::new());
    response.auth_methods = vec![acp::AuthMethod::Terminal(
        acp::AuthMethodTerminal::new(acp::AuthMethodId::new("term"), "终端登录")
            .args(vec!["--login".to_owned()]),
    )];
    let mut value = serde_json::to_value(&response).unwrap();
    value["_meta"] = json!({"steering": {"supported": true}});
    let capabilities = parse_capabilities(&value);
    assert!(capabilities.load_session);
    assert!(capabilities.close);
    assert!(!capabilities.resume);
    assert!(capabilities.mcp_http);
    assert!(capabilities.steering);
    let methods = parse_auth_methods(value.get("authMethods"));
    assert_eq!(methods.len(), 1);
    assert!(methods[0].is_terminal() && methods[0].supported());
    assert_eq!(methods[0].id, "term");
    assert_eq!(methods[0].args, vec!["--login".to_owned()]);
}

#[tokio::test]
async fn official_new_session_response_drives_both_mode_writings() {
    let mut response = acp::NewSessionResponse::new("session-1");
    response.modes = Some(acp::SessionModeState::new(
        "plan",
        vec![
            acp::SessionMode::new("plan", "Plan"),
            acp::SessionMode::new("workspace-write", "Workspace write"),
        ],
    ));
    let value = serde_json::to_value(&response).unwrap();
    let (method, params) = mode::request(&value, "session-1", "workspace-write").unwrap();
    assert_eq!(method, "session/set_mode");
    let request = as_official::<acp::SetSessionModeRequest>(&params);
    assert_eq!(request.session_id.0.as_ref(), "session-1");
    assert_eq!(request.mode_id.0.as_ref(), "workspace-write");
    assert!(mode::topic_request(&value, "session-1", "plan").is_some());
    assert!(mode::topic_request(&value, "session-1", "nope").is_none());

    // dsh 0.2.2 的写法：没有 modes，只有类别为 mode 的 configOptions。这面在官方
    // crate 还是 unstable_session_options 特性，没有稳定承诺。
    let option: acp::SessionConfigOption = serde_json::from_value(json!({
        "id": "mode",
        "name": "工作模式",
        "category": "mode",
        "type": "select",
        "currentValue": "plan",
        "options": [
            {"value": "plan", "name": "Plan"},
            {"value": "workspace-write", "name": "Workspace write"}
        ]
    }))
    .unwrap();
    let mut config = acp::NewSessionResponse::new("session-1");
    config.config_options = Some(vec![option]);
    let value = serde_json::to_value(&config).unwrap();
    let (method, params) = mode::request(&value, "session-1", "plan").unwrap();
    assert_eq!(method, "session/set_config_option");
    let request = as_official::<acp::SetSessionConfigOptionRequest>(&params);
    assert_eq!(request.session_id.0.as_ref(), "session-1");
    assert_eq!(request.config_id.0.as_ref(), "mode");
    assert_eq!(serde_json::to_value(&request.value).unwrap(), json!("plan"));
    assert!(mode::topic_request(&value, "session-1", "plan").is_some());
}

#[tokio::test]
async fn official_updates_flow_through_the_real_read_loop() {
    let agent = FakeAgent::start(Scenario::Manual).await;
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
    {
        // prompt 只在被 poll 时才把请求发出去：select 一边推进它一边等 FakeAgent 记下挂起的轮。
        let mut prompt = std::pin::pin!(client.prompt(
            "session-1",
            vec![json!({"type": "text", "text": "hi"})],
            1024,
            Duration::from_secs(30),
            zh(),
        ));
        let mut wait = std::pin::pin!(async {
            while agent.pending_count() == 0 {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        });
        let mut sent = false;
        loop {
            tokio::select! {
                _ = &mut wait, if !sent => {
                    sent = true;
                    let request_id = request_of(&agent, "session/prompt").await["id"].clone();
                    let mut chunk = acp::ContentChunk::new(acp::ContentBlock::Text(
                        acp::TextContent::new("官方块".to_owned()),
                    ));
                    chunk.message_id = Some("m-c1".to_owned());
                    let update =
                        acp::SessionNotification::new("session-1", acp::SessionUpdate::AgentMessageChunk(chunk));
                    agent
                        .send_agent_frame(json!({
                            "jsonrpc": "2.0",
                            "method": "session/update",
                            "params": serde_json::to_value(&update).unwrap()
                        }))
                        .await;
                    let response = acp::PromptResponse::new(acp::StopReason::EndTurn);
                    agent
                        .send_agent_frame(json!({
                            "jsonrpc": "2.0",
                            "id": request_id,
                            "result": serde_json::to_value(&response).unwrap()
                        }))
                        .await;
                }
                result = &mut prompt => {
                    assert_eq!(result.unwrap().text, "官方块");
                    break;
                }
            }
        }
    }
    drop(client);
    agent.finish().await;
}

#[tokio::test]
async fn official_permission_request_merges_and_replies_official_outcome() {
    let (agent, client) = connect_normal().await;
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
    let (ask_tx, mut ask_rx) = tokio::sync::mpsc::channel(4);
    client.register_permissions("session-1", ask_tx).await;

    // 先来一条官方 tool_call 通知；权限请求只带增量（toolCallId）时靠它合并。
    let tool_call: acp::SessionUpdate = serde_json::from_value(json!({
        "sessionUpdate": "tool_call",
        "toolCallId": "call-perm",
        "title": "改 /workspace/a.txt",
        "kind": "edit",
        "status": "pending",
        "locations": [{"path": "/workspace/a.txt"}]
    }))
    .unwrap();
    let notification = acp::SessionNotification::new("session-1", tool_call);
    agent
        .send_agent_frame(json!({
            "jsonrpc": "2.0",
            "method": "session/update",
            "params": serde_json::to_value(&notification).unwrap()
        }))
        .await;
    let permission: acp::RequestPermissionRequest = serde_json::from_value(json!({
        "sessionId": "session-1",
        "toolCall": {"toolCallId": "call-perm"},
        "options": [
            {"optionId": "allow_once", "kind": "allow_once", "name": "允许"},
            {"optionId": "reject_once", "kind": "reject_once", "name": "拒绝"}
        ]
    }))
    .unwrap();
    agent
        .send_agent_frame(json!({
            "jsonrpc": "2.0",
            "id": 8000,
            "method": "session/request_permission",
            "params": serde_json::to_value(&permission).unwrap()
        }))
        .await;
    let ask = bounded(ask_rx.recv(), "等权限请求").await.unwrap();
    assert_eq!(ask.session_id, "session-1");
    assert_eq!(ask.pending.locations, vec!["/workspace/a.txt".to_owned()]);
    assert_eq!(ask.pending.kind.as_deref(), Some("edit"));
    // option_ids 是桥生成的 opaque choice_id → ACP optionId；键不是 ACP 的。
    assert_eq!(ask.pending.option_ids.len(), 2);
    assert!(
        ask.pending
            .option_ids
            .values()
            .any(|option_id| option_id == "allow_once")
    );
    ask.reply
        .send(Ok(crate::permission::PermissionResult::Selected {
            option_id: "allow_once".to_owned(),
            reason: None,
        }))
        .unwrap();
    let outcome: acp::RequestPermissionResponse =
        as_official(&response_of(&agent, &json!(8000)).await["result"]);
    assert!(matches!(
        &outcome.outcome,
        acp::RequestPermissionOutcome::Selected(selected)
            if selected.option_id.0.as_ref() == "allow_once"
    ));

    // 第二条不回答：桥按取消回，应答同样是官方形状。
    agent
        .send_agent_frame(json!({
            "jsonrpc": "2.0",
            "id": 8001,
            "method": "session/request_permission",
            "params": serde_json::to_value(&permission).unwrap()
        }))
        .await;
    let ask = bounded(ask_rx.recv(), "等第二条权限请求").await.unwrap();
    drop(ask);
    let outcome: acp::RequestPermissionResponse =
        as_official(&response_of(&agent, &json!(8001)).await["result"]);
    assert_eq!(outcome.outcome, acp::RequestPermissionOutcome::Cancelled);
    drop(client);
    agent.finish().await;
}

#[tokio::test]
async fn elicitation_reply_parses_with_official_types() {
    let (agent, client) = connect_normal().await;
    let request: acp::CreateElicitationRequest = serde_json::from_value(json!({
        "mode": "url",
        "requestId": 7000,
        "elicitationId": "login-1",
        "url": "https://auth.example/idle",
        "message": "空闲时的引导"
    }))
    .unwrap();
    agent
        .send_agent_frame(json!({
            "jsonrpc": "2.0",
            "id": 7000,
            "method": "elicitation/create",
            "params": serde_json::to_value(&request).unwrap()
        }))
        .await;
    let response = bounded(response_of(&agent, &json!(7000)), "等 elicitation 应答").await;
    let reply: acp::CreateElicitationResponse = as_official(&response["result"]);
    assert_eq!(reply.action, acp::ElicitationAction::Cancel);
    drop(client);
    agent.finish().await;
}
