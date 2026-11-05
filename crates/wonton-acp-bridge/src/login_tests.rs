use super::*;
use crate::acp::test_support::*;
use crate::login::LoginSink;
use serde_json::json;

#[derive(Clone, Default)]
struct MemorySink(Arc<std::sync::Mutex<Vec<String>>>);

impl MemorySink {
    fn texts(&self) -> Vec<String> {
        self.0.lock().unwrap().clone()
    }
}

impl LoginSink for MemorySink {
    async fn send_login_text(&self, text: &str) -> crate::error::Result<()> {
        self.0.lock().unwrap().push(text.to_owned());
        Ok(())
    }
}

#[tokio::test]
async fn broken_login_method_is_skipped_without_failing_connect() {
    use crate::login::LoginFlow;
    let agent = FakeAgent::start(Scenario::LoginMixed).await;
    let client = Arc::new(
        bounded(AcpClient::connect(&agent.socket, broker()), "initialize")
            .await
            .unwrap(),
    );
    let methods = client.auth_methods();
    assert_eq!(methods.len(), 2);
    assert_eq!(methods[0].id, "good");
    assert_eq!(methods[1].id, "env-method");
    // 列表命令照常工作，只显示能解析的方式。
    let (mut login, mut events) = LoginFlow::new();
    let sink = MemorySink::default();
    bounded(
        login.receive("/wonton login", &sink, &client, &agent.socket, false),
        "list methods",
    )
    .await
    .unwrap();
    let texts = sink.texts().join("\n");
    assert!(texts.contains("/wonton login good"));
    assert!(!texts.contains("broken"));
    assert!(events.try_recv().is_err());
    drop(client);
    agent.finish().await;
}

#[tokio::test]
async fn url_login_completes_through_chat_commands() {
    use crate::login::LoginFlow;
    let agent = FakeAgent::start(Scenario::LoginUrl).await;
    let client = Arc::new(
        bounded(AcpClient::connect(&agent.socket, broker()), "initialize")
            .await
            .unwrap(),
    );
    let error = bounded(
        client.new_session("/workspace", "http://mcp.wonton.internal/mcp", "plan"),
        "gated session",
    )
    .await
    .unwrap_err();
    assert!(matches!(error, Error::AuthenticationRequired));

    let (mut login, mut events) = LoginFlow::new();
    let sink = MemorySink::default();
    bounded(
        login.receive("/wonton login", &sink, &client, &agent.socket, false),
        "list methods",
    )
    .await
    .unwrap();
    assert!(
        sink.texts()
            .iter()
            .any(|text| text.contains("/wonton login url-method"))
    );

    bounded(
        login.receive(
            "/wonton login url-method",
            &sink,
            &client,
            &agent.socket,
            false,
        ),
        "start login",
    )
    .await
    .unwrap();
    while !sink
        .texts()
        .iter()
        .any(|text| text.contains("https://auth.example/login"))
    {
        let event = bounded(events.recv(), "login event").await.unwrap();
        bounded(login.event(event, &sink), "deliver event")
            .await
            .unwrap();
    }
    assert!(
        sink.texts()
            .iter()
            .any(|text| text.contains("/wonton login accept"))
    );

    bounded(
        login.receive("/wonton login accept", &sink, &client, &agent.socket, false),
        "accept",
    )
    .await
    .unwrap();
    let event = bounded(events.recv(), "finished event").await.unwrap();
    let reconnect = bounded(login.event(event, &sink), "finish").await.unwrap();
    assert!(!reconnect);

    let session = bounded(
        client.new_session("/workspace", "http://mcp.wonton.internal/mcp", "plan"),
        "session after login",
    )
    .await
    .unwrap();
    assert_eq!(session, "session-1");
    drop(client);
    agent.finish().await;
}

#[tokio::test]
async fn terminal_login_forwards_code_and_reconnects() {
    use crate::login::{LoginEvent, LoginFlow};
    let agent = FakeAgent::start(Scenario::LoginTerminal).await;
    let client = Arc::new(
        bounded(AcpClient::connect(&agent.socket, broker()), "initialize")
            .await
            .unwrap(),
    );
    let error = bounded(
        client.new_session("/workspace", "http://mcp.wonton.internal/mcp", "plan"),
        "gated session",
    )
    .await
    .unwrap_err();
    assert!(matches!(error, Error::AuthenticationRequired));

    let (mut login, mut events) = LoginFlow::new();
    let sink = MemorySink::default();
    bounded(
        login.receive("/wonton login term", &sink, &client, &agent.socket, false),
        "start terminal login",
    )
    .await
    .unwrap();
    let event = bounded(events.recv(), "terminal output").await.unwrap();
    let LoginEvent::Output(link) = &event else {
        panic!("期待终端登录输出链接");
    };
    assert_eq!(link, "https://auth.example/device");
    bounded(login.event(event, &sink), "deliver link")
        .await
        .unwrap();

    bounded(
        login.receive("code-42", &sink, &client, &agent.socket, false),
        "send code",
    )
    .await
    .unwrap();
    let reconnect = loop {
        let event = bounded(events.recv(), "login event").await.unwrap();
        if let LoginEvent::Output(_) = &event {
            bounded(login.event(event, &sink), "deliver output")
                .await
                .unwrap();
            continue;
        }
        break bounded(login.event(event, &sink), "finish").await.unwrap();
    };
    assert!(reconnect);

    // 登录态留在 Agent 容器的持久目录里；桥重连后不再要求登录。
    let reconnected = bounded(AcpClient::connect(&agent.socket, broker()), "reconnect")
        .await
        .unwrap();
    let session = bounded(
        reconnected.new_session("/workspace", "http://mcp.wonton.internal/mcp", "plan"),
        "session after reconnect",
    )
    .await
    .unwrap();
    assert_eq!(session, "session-1");
    drop(reconnected);
    drop(client);
    agent.finish().await;
}

#[tokio::test]
async fn idle_url_elicitation_is_cancelled_immediately() {
    use crate::login::LoginFlow;
    let agent = FakeAgent::start(Scenario::IdleElicit).await;
    let client = Arc::new(
        bounded(AcpClient::connect(&agent.socket, broker()), "initialize")
            .await
            .unwrap(),
    );
    let answered = loop {
        if let Some(line) = agent
            .responses()
            .await
            .iter()
            .find(|message| message["id"] == json!(7_000))
            .cloned()
        {
            break line;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    assert_eq!(answered["result"]["action"], "cancel");
    // 空闲时留下的请求不挡后面的登录：列表照常。
    let (mut login, _events) = LoginFlow::new();
    let sink = MemorySink::default();
    bounded(
        login.receive("/wonton login", &sink, &client, &agent.socket, false),
        "list methods",
    )
    .await
    .unwrap();
    assert!(sink.texts().iter().any(|text| text.contains("url-method")));
    drop(client);
    agent.finish().await;
}

#[tokio::test]
async fn unsupported_login_method_is_hidden_and_harmless() {
    use crate::login::LoginFlow;
    let agent = FakeAgent::start(Scenario::LoginMixed).await;
    let client = Arc::new(
        bounded(AcpClient::connect(&agent.socket, broker()), "initialize")
            .await
            .unwrap(),
    );
    let (mut login, _events) = LoginFlow::new();
    let sink = MemorySink::default();
    bounded(
        login.receive("/wonton login", &sink, &client, &agent.socket, false),
        "list methods",
    )
    .await
    .unwrap();
    let list = sink.texts().join("\n");
    assert!(list.contains("/wonton login good"));
    assert!(!list.contains("env-method"));
    // 选一个存在但不受支持的方式：只回聊天提示，不返回错误。
    bounded(
        login.receive(
            "/wonton login env-method",
            &sink,
            &client,
            &agent.socket,
            false,
        ),
        "unsupported method",
    )
    .await
    .unwrap();
    assert!(sink.texts().iter().any(|text| text.contains("还不支持")));
    assert!(!login.active());
    // 再选一个不存在的 ID，同样只是提示。
    bounded(
        login.receive("/wonton login nope", &sink, &client, &agent.socket, false),
        "unknown method",
    )
    .await
    .unwrap();
    assert!(
        sink.texts()
            .iter()
            .any(|text| text.contains("没有提供这种登录方式"))
    );
    drop(client);
    agent.finish().await;
}
