use super::*;
use crate::acp::test_support::*;
use crate::l10n::zh;
use crate::login::LoginSink;
use serde_json::json;

/// 连上要求登录的 Agent：`new_session` 被拒（`-32000`），登录完成后才能建会话。
async fn gated_client(scenario: Scenario) -> (FakeAgent, Arc<AcpClient>) {
    let agent = FakeAgent::start(scenario).await;
    let client = Arc::new(
        bounded(AcpClient::connect(&agent.socket, broker()), "initialize")
            .await
            .unwrap(),
    );
    let error = bounded(
        client.new_session(
            "/workspace",
            "http://mcp.wonton.internal/mcp",
            "plan",
            None,
            &origin(None),
        ),
        "gated session",
    )
    .await
    .unwrap_err();
    assert!(matches!(error, Error::AuthenticationRequired));
    (agent, client)
}

#[derive(Clone, Default)]
struct MemorySink {
    texts: Arc<std::sync::Mutex<Vec<String>>>,
    redactions: Arc<std::sync::Mutex<Vec<(String, String)>>>,
    fail_redact: bool,
}

impl MemorySink {
    fn texts(&self) -> Vec<String> {
        self.texts.lock().unwrap().clone()
    }

    fn redactions(&self) -> Vec<(String, String)> {
        self.redactions.lock().unwrap().clone()
    }

    fn failing_redact() -> Self {
        Self {
            fail_redact: true,
            ..Default::default()
        }
    }
}

impl LoginSink for MemorySink {
    async fn send_login_text(&self, text: &str) -> crate::error::Result<()> {
        self.texts.lock().unwrap().push(text.to_owned());
        Ok(())
    }

    async fn redact(&self, event_id: &str, reason: &str) -> crate::error::Result<()> {
        if self.fail_redact {
            return Err(Error::Matrix(Detail::data("没有权限撤回这条消息")));
        }
        self.redactions
            .lock()
            .unwrap()
            .push((event_id.to_owned(), reason.to_owned()));
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
        login.receive(
            "/wonton login",
            "$msg:example.org",
            &sink,
            &client,
            &agent.socket,
            false,
            zh(),
        ),
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
    let (agent, client) = gated_client(Scenario::LoginUrl).await;
    let (mut login, mut events) = LoginFlow::new();
    let sink = MemorySink::default();
    bounded(
        login.receive(
            "/wonton login",
            "$msg:example.org",
            &sink,
            &client,
            &agent.socket,
            false,
            zh(),
        ),
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
            "$msg:example.org",
            &sink,
            &client,
            &agent.socket,
            false,
            zh(),
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
        bounded(login.event(event, &sink, zh()), "deliver event")
            .await
            .unwrap();
    }
    assert!(
        sink.texts()
            .iter()
            .any(|text| text.contains("/wonton login accept"))
    );

    bounded(
        login.receive(
            "/wonton login accept",
            "$msg:example.org",
            &sink,
            &client,
            &agent.socket,
            false,
            zh(),
        ),
        "accept",
    )
    .await
    .unwrap();
    let event = bounded(events.recv(), "finished event").await.unwrap();
    let reconnect = bounded(login.event(event, &sink, zh()), "finish")
        .await
        .unwrap();
    assert!(!reconnect);

    let session = bounded(
        client.new_session(
            "/workspace",
            "http://mcp.wonton.internal/mcp",
            "plan",
            None,
            &origin(None),
        ),
        "session after login",
    )
    .await
    .unwrap();
    assert_eq!(session.session_id, "session-2");
    drop(client);
    agent.finish().await;
}

#[tokio::test]
async fn terminal_login_forwards_code_and_reconnects() {
    use crate::login::{LoginEvent, LoginFlow};
    let (agent, client) = gated_client(Scenario::LoginTerminal).await;
    let (mut login, mut events) = LoginFlow::new();
    let sink = MemorySink::default();
    bounded(
        login.receive(
            "/wonton login term",
            "$msg:example.org",
            &sink,
            &client,
            &agent.socket,
            false,
            zh(),
        ),
        "start terminal login",
    )
    .await
    .unwrap();
    let event = bounded(events.recv(), "terminal output").await.unwrap();
    let LoginEvent::Output(link) = &event else {
        panic!("期待终端登录输出链接");
    };
    assert_eq!(link, "https://auth.example/device");
    bounded(login.event(event, &sink, zh()), "deliver link")
        .await
        .unwrap();

    bounded(
        login.receive(
            "code-42",
            "$key:example.org",
            &sink,
            &client,
            &agent.socket,
            false,
            zh(),
        ),
        "send code",
    )
    .await
    .unwrap();
    let reconnect = loop {
        let event = bounded(events.recv(), "login event").await.unwrap();
        if let LoginEvent::Output(_) = &event {
            bounded(login.event(event, &sink, zh()), "deliver output")
                .await
                .unwrap();
            continue;
        }
        break bounded(login.event(event, &sink, zh()), "finish")
            .await
            .unwrap();
    };
    assert!(reconnect);

    // 登录态留在 Agent 容器的持久目录里；桥重连后不再要求登录。
    let reconnected = bounded(AcpClient::connect(&agent.socket, broker()), "reconnect")
        .await
        .unwrap();
    let session = bounded(
        reconnected.new_session(
            "/workspace",
            "http://mcp.wonton.internal/mcp",
            "plan",
            None,
            &origin(None),
        ),
        "session after reconnect",
    )
    .await
    .unwrap();
    assert_eq!(session.session_id, "session-1");
    drop(reconnected);
    drop(client);
    agent.finish().await;
}

#[tokio::test]
async fn terminal_login_redacts_the_forwarded_key_message() {
    use crate::login::{LoginEvent, LoginFlow};
    let agent = FakeAgent::start(Scenario::LoginTerminal).await;
    let client = Arc::new(
        bounded(AcpClient::connect(&agent.socket, broker()), "initialize")
            .await
            .unwrap(),
    );
    let (mut login, mut events) = LoginFlow::new();
    let sink = MemorySink::default();
    bounded(
        login.receive(
            "/wonton login term",
            "$msg:example.org",
            &sink,
            &client,
            &agent.socket,
            false,
            zh(),
        ),
        "start terminal login",
    )
    .await
    .unwrap();
    let event = bounded(events.recv(), "terminal output").await.unwrap();
    let LoginEvent::Output(_) = &event else {
        panic!("期待终端登录输出链接");
    };
    bounded(login.event(event, &sink, zh()), "deliver link")
        .await
        .unwrap();
    // 开始登录不撤回任何消息。
    assert!(sink.redactions().is_empty());
    bounded(
        login.receive(
            "code-42",
            "$key:example.org",
            &sink,
            &client,
            &agent.socket,
            false,
            zh(),
        ),
        "send code",
    )
    .await
    .unwrap();
    // 只有转交给 Agent 的那一条被撤回。
    assert_eq!(
        sink.redactions(),
        vec![("$key:example.org".to_owned(), redact_reason(zh()))]
    );
    drop(client);
    agent.finish().await;
}

#[tokio::test]
async fn failed_redaction_tells_the_user_to_delete_the_key() {
    use crate::login::{LoginEvent, LoginFlow};
    let agent = FakeAgent::start(Scenario::LoginTerminal).await;
    let client = Arc::new(
        bounded(AcpClient::connect(&agent.socket, broker()), "initialize")
            .await
            .unwrap(),
    );
    let (mut login, mut events) = LoginFlow::new();
    let sink = MemorySink::failing_redact();
    bounded(
        login.receive(
            "/wonton login term",
            "$msg:example.org",
            &sink,
            &client,
            &agent.socket,
            false,
            zh(),
        ),
        "start terminal login",
    )
    .await
    .unwrap();
    let event = bounded(events.recv(), "terminal output").await.unwrap();
    let LoginEvent::Output(_) = &event else {
        panic!("期待终端登录输出链接");
    };
    bounded(login.event(event, &sink, zh()), "deliver link")
        .await
        .unwrap();
    bounded(
        login.receive(
            "code-42",
            "$key:example.org",
            &sink,
            &client,
            &agent.socket,
            false,
            zh(),
        ),
        "send code",
    )
    .await
    .unwrap();
    // 撤回失败不能静默：告诉用户消息里还有密钥。
    assert!(
        sink.texts()
            .iter()
            .any(|text| text.contains("这条消息里有登录用的密钥，请自己删掉它。"))
    );
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
        login.receive(
            "/wonton login",
            "$msg:example.org",
            &sink,
            &client,
            &agent.socket,
            false,
            zh(),
        ),
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
        login.receive(
            "/wonton login",
            "$msg:example.org",
            &sink,
            &client,
            &agent.socket,
            false,
            zh(),
        ),
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
            "$msg:example.org",
            &sink,
            &client,
            &agent.socket,
            false,
            zh(),
        ),
        "unsupported method",
    )
    .await
    .unwrap();
    assert!(sink.texts().iter().any(|text| text.contains("还不支持")));
    assert!(!login.active());
    // 再选一个不存在的 ID，同样只是提示。
    bounded(
        login.receive(
            "/wonton login nope",
            "$msg:example.org",
            &sink,
            &client,
            &agent.socket,
            false,
            zh(),
        ),
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

#[tokio::test]
async fn a_group_login_moves_to_the_direct_chat() {
    use crate::login::{LoginEvent, LoginFlow};
    let agent = FakeAgent::start(Scenario::LoginTerminal).await;
    let client = Arc::new(
        bounded(AcpClient::connect(&agent.socket, broker()), "initialize")
            .await
            .unwrap(),
    );
    let (mut login, mut events) = LoginFlow::new();
    let group = MemorySink::default();
    let direct = MemorySink::default();
    let chat = LoginChat {
        current: &group,
        direct: Some(&direct),
        group: true,
    };
    bounded(
        login.receive_from(
            "/wonton login term",
            "$cmd:example.org",
            &chat,
            &client,
            &agent.socket,
            false,
            zh(),
        ),
        "start login in group",
    )
    .await
    .unwrap();
    // 群里只留一句，方式提示在私聊。
    assert_eq!(group.texts(), ["登录在我们的私聊里进行。"]);

    // 登录事件由私聊的处理循环投递。
    let event = bounded(events.recv(), "terminal output").await.unwrap();
    let LoginEvent::Output(link) = &event else {
        panic!("期待终端登录提示");
    };
    let link = link.clone();
    assert_eq!(link, "https://auth.example/device");
    bounded(login.event(event, &direct, zh()), "deliver prompt")
        .await
        .unwrap();
    assert!(direct.texts().iter().any(|text| text.contains(&link)));

    // 群里再说普通的话照常走 ACP，不当成 key。
    assert!(
        !bounded(
            login.receive_from(
                "普通消息",
                "$chat:example.org",
                &chat,
                &client,
                &agent.socket,
                false,
                zh(),
            ),
            "group chat",
        )
        .await
        .unwrap()
    );

    // 私聊里回 key：照旧转交并撤回那条消息；再取消。
    finish_login_in_direct(&mut login, &direct, &group, &client, &agent.socket).await;
}

/// 私聊里交授权码并取消登录。
async fn finish_login_in_direct(
    login: &mut LoginFlow,
    direct: &MemorySink,
    group: &MemorySink,
    client: &Arc<AcpClient>,
    socket: &std::path::Path,
) {
    let dm = LoginChat {
        current: direct,
        direct: None,
        group: false,
    };
    bounded(
        login.receive_from(
            "code-42",
            "$key:example.org",
            &dm,
            client,
            socket,
            false,
            zh(),
        ),
        "send code in direct chat",
    )
    .await
    .unwrap();
    assert_eq!(
        direct.redactions(),
        [("$key:example.org".to_owned(), redact_reason(zh()))]
    );
    assert_eq!(group.texts(), ["登录在我们的私聊里进行。"]);

    bounded(
        login.receive_from(
            "/wonton login cancel",
            "$cancel:example.org",
            &dm,
            client,
            socket,
            false,
            zh(),
        ),
        "cancel in direct chat",
    )
    .await
    .unwrap();
    assert!(!login.active());
    assert!(
        direct
            .texts()
            .iter()
            .any(|text| text.contains("登录已取消"))
    );
}

#[tokio::test]
async fn a_group_login_without_a_direct_chat_says_where_to_go() {
    use crate::login::LoginFlow;
    let agent = FakeAgent::start(Scenario::LoginTerminal).await;
    let client = Arc::new(
        bounded(AcpClient::connect(&agent.socket, broker()), "initialize")
            .await
            .unwrap(),
    );
    let (mut login, _events) = LoginFlow::new();
    let group = MemorySink::default();
    let chat = LoginChat {
        current: &group,
        direct: None,
        group: true,
    };
    bounded(
        login.receive_from(
            "/wonton login",
            "$cmd:example.org",
            &chat,
            &client,
            &agent.socket,
            false,
            zh(),
        ),
        "list in group without direct chat",
    )
    .await
    .unwrap();
    assert_eq!(
        group.texts(),
        ["找不到和你的私聊，请在私聊里发 /wonton login。"]
    );
    assert!(!login.active());
    drop(client);
    agent.finish().await;
}
