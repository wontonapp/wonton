use std::net::Ipv4Addr;
use std::{
    pin::Pin,
    task::{Context, Poll},
};

use tokio::{io::AsyncBufReadExt, io::AsyncReadExt, net::TcpListener, task::JoinHandle};

use super::*;
use crate::acp::test_support::*;
use crate::l10n::zh;
use tokio::io::{AsyncRead, ReadBuf};
use wonton_protocol::{ORIGIN_ROOM_HEADER, ORIGIN_THREAD_HEADER};

/// 假核心的权限接口：每个转发来的权限请求都回同一个结果。
struct FakeCore {
    url: String,
    task: JoinHandle<()>,
}

impl FakeCore {
    async fn replying(outcome: Value) -> Self {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(serve_permissions(listener, outcome));
        Self {
            url: format!("http://{address}/mcp"),
            task,
        }
    }

    fn broker(&self) -> PermissionBroker {
        PermissionBroker::new(&self.url).unwrap()
    }

    fn finish(&self) {
        self.task.abort();
    }
}

async fn serve_permissions(listener: TcpListener, outcome: Value) {
    loop {
        let Ok((mut stream, _)) = listener.accept().await else {
            return;
        };
        let outcome = outcome.clone();
        tokio::spawn(async move {
            let mut reader = BufReader::new(&mut stream);
            let mut content_length = 0;
            let mut request_line = String::new();
            if reader.read_line(&mut request_line).await.is_err() {
                return;
            }
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).await.is_err() {
                    return;
                }
                if line == "\r\n" || line.is_empty() {
                    break;
                }
                if let Some((name, value)) = line.split_once(':')
                    && name.eq_ignore_ascii_case("content-length")
                {
                    content_length = value.trim().parse().unwrap_or(0);
                }
            }
            let mut body = vec![0; content_length];
            if reader.read_exact(&mut body).await.is_err() {
                return;
            }
            let outcome = outcome.to_string();
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{outcome}",
                outcome.len()
            );
            let _ = stream.write_all(response.as_bytes()).await;
        });
    }
}

/// 等到这个权限请求收到应答（最久 5 秒，`bounded` 兜底）。
async fn permission_response(agent: &FakeAgent, id: &Value) -> Vec<Value> {
    bounded(
        async {
            loop {
                let responses = agent.responses_for(id).await;
                if !responses.is_empty() {
                    return responses;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        },
        "等权限应答",
    )
    .await
}

/// 挂起中的一轮。
type PromptTask = JoinHandle<Result<PromptResult>>;

/// 起假 Agent + 假核心，开好会话、挂起一轮 prompt，返回挂起的一轮。
async fn started_with(outcome: Value) -> (FakeAgent, FakeCore, AcpClient, PromptTask) {
    let agent = FakeAgent::start(Scenario::Manual).await;
    let core = FakeCore::replying(outcome).await;
    let client = bounded(
        AcpClient::connect(&agent.socket, core.broker()),
        "initialize",
    )
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
    let prompt = {
        let client = client.clone();
        tokio::spawn(async move {
            client
                .prompt(
                    "session-1",
                    vec![json!({"type": "text", "text": "hi"})],
                    None,
                    1024,
                    Duration::from_secs(2),
                    zh(),
                )
                .await
        })
    };
    bounded(
        async {
            while agent.pending_count() == 0 {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        },
        "等一轮挂起",
    )
    .await;
    (agent, core, client, prompt)
}

#[tokio::test]
async fn fake_agent_round_trip_sets_http_mcp_and_mode() {
    let agent = FakeAgent::start(Scenario::Normal).await;
    let client = bounded(AcpClient::connect(&agent.socket, broker()), "initialize")
        .await
        .unwrap();
    let session = bounded(
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
    assert_eq!(session.session_id, "session-1");
    let result = bounded(
        client.prompt(
            "session-1",
            vec![json!({"type": "text", "text": "hi"})],
            None,
            1024,
            Duration::from_secs(2),
            zh(),
        ),
        "prompt",
    )
    .await
    .unwrap();
    assert_eq!(result.text, "hello");
    assert_eq!(
        bounded(
            client.steer(
                "session-1",
                vec![json!({"type": "text", "text": "continue"})]
            ),
            "steering"
        )
        .await
        .unwrap(),
        SteerOutcome::Injected
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
async fn agent_without_modes_gets_a_session_without_set_mode() {
    let agent = FakeAgent::start(Scenario::NoModes).await;
    let client = bounded(AcpClient::connect(&agent.socket, broker()), "initialize")
        .await
        .unwrap();
    let session = bounded(
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
    assert_eq!(session.session_id, "session-1");
    let requests = agent.requests().await;
    assert!(
        !requests
            .iter()
            .any(|request| request["method"] == "session/set_mode")
    );
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
        client.prompt(
            "session-1",
            vec![json!({"type": "text", "text": "long"})],
            None,
            1024,
            Duration::from_millis(50),
            zh(),
        ),
        "cancelled prompt",
    )
    .await;
    assert!(matches!(first, Err(Error::TransportUnavailable(_))));
    let result = bounded(
        client.prompt(
            "session-1",
            vec![json!({"type": "text", "text": "new"})],
            None,
            1024,
            Duration::from_secs(2),
            zh(),
        ),
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
        client.load_or_resume(
            "session-1",
            "/workspace",
            "http://mcp.wonton.internal/mcp",
            &origin(None),
            "plan",
        ),
        "session/load",
    )
    .await
    .unwrap();
    let result = bounded(
        client.prompt(
            "session-1",
            vec![json!({"type": "text", "text": "next"})],
            None,
            1024,
            Duration::from_secs(2),
            zh(),
        ),
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
            client.load_or_resume(
                "session-1",
                "/workspace",
                "http://mcp.wonton.internal/mcp",
                &origin(None),
                "plan",
            ),
            "session/load",
        )
        .await;
        assert!(matches!(result, Err(Error::SessionUnavailable(_))));
        drop(client);
        agent.finish().await;
    }
}

#[tokio::test]
async fn prompt_errors_keep_their_categories() {
    for (scenario, transport) in [
        (Scenario::PromptTransportError, true),
        (Scenario::PromptUpstreamError, false),
    ] {
        let agent = FakeAgent::start(scenario).await;
        let client = bounded(AcpClient::connect(&agent.socket, broker()), "initialize")
            .await
            .unwrap();
        let error = bounded(
            client.prompt(
                "session-1",
                vec![json!({"type": "text", "text": "hi"})],
                None,
                1024,
                Duration::from_secs(2),
                zh(),
            ),
            "prompt",
        )
        .await
        .unwrap_err();
        if transport {
            assert!(
                matches!(error, Error::TransportUnavailable(_)),
                "got {error}"
            );
        } else {
            assert!(matches!(error, Error::Upstream(_)), "got {error}");
        }
        drop(client);
        agent.finish().await;
    }
}

#[tokio::test]
async fn prompt_errors_show_the_reason_from_data() {
    let agent = FakeAgent::start(Scenario::PromptErrorData).await;
    let client = bounded(AcpClient::connect(&agent.socket, broker()), "initialize")
        .await
        .unwrap();
    let prompt_error = || {
        bounded(
            client.prompt(
                "session-1",
                vec![json!({"type": "text", "text": "hi"})],
                None,
                1024,
                Duration::from_secs(2),
                zh(),
            ),
            "prompt",
        )
    };
    // data.details 接在笼统的 message 后面，用户能看到要做什么。
    let shown = prompt_error().await.unwrap_err().to_string();
    assert!(
        shown.contains("Internal error")
            && shown.contains("No model selected. Use /login 选择模型。"),
        "用户看到的：{shown}"
    );
    // data 本身是字符串。
    let shown = prompt_error().await.unwrap_err().to_string();
    assert!(
        shown.contains("后台正在编译，请稍后再试"),
        "用户看到的：{shown}"
    );
    // detail 和 message 相同时不重复。
    assert_eq!(
        prompt_error().await.unwrap_err().to_string(),
        "ACP 上游出错：同一个原因"
    );
    // 其余形状照旧：不认识的 data 不往上接。
    assert_eq!(
        prompt_error().await.unwrap_err().to_string(),
        "ACP 上游出错：Internal error"
    );
    // 整段截到 2 KiB，不切开多字节字符。
    if let Error::Upstream(text) = prompt_error().await.unwrap_err() {
        let text = text.format(zh());
        assert!(
            text.len() > 2000 && text.len() <= 2048,
            "截断后 {} 字节",
            text.len()
        );
        assert!(text.starts_with("Internal error\n"));
    } else {
        panic!("期待 Upstream 错误");
    }
    drop(client);
    agent.finish().await;
}

#[tokio::test]
async fn session_setup_reports_the_origin_session_in_mcp_headers() {
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
    bounded(
        client.new_session(
            "/workspace",
            "http://mcp.wonton.internal/mcp",
            "plan",
            None,
            &origin(Some("$thread-1")),
        ),
        "session/new",
    )
    .await
    .unwrap();
    bounded(
        client.load_or_resume(
            "session-1",
            "/workspace",
            "http://mcp.wonton.internal/mcp",
            &origin(Some("$thread-2")),
            "plan",
        ),
        "session/load",
    )
    .await
    .unwrap();
    let requests = agent.requests().await;
    let headers_of = |method: &str, index: usize| -> Value {
        requests
            .iter()
            .filter(|request| request["method"] == method)
            .nth(index)
            .unwrap()["params"]["mcpServers"][0]["headers"]
            .clone()
    };
    // 主对话只报房间，话题会话带话题根（ADR-0016）。
    assert_eq!(
        headers_of("session/new", 0),
        json!([{"name": ORIGIN_ROOM_HEADER, "value": "!room:wonton.internal"}])
    );
    assert_eq!(
        headers_of("session/new", 1),
        json!([
            {"name": ORIGIN_ROOM_HEADER, "value": "!room:wonton.internal"},
            {"name": ORIGIN_THREAD_HEADER, "value": "$thread-1"}
        ])
    );
    assert_eq!(
        headers_of("session/load", 0),
        json!([
            {"name": ORIGIN_ROOM_HEADER, "value": "!room:wonton.internal"},
            {"name": ORIGIN_THREAD_HEADER, "value": "$thread-2"},
            // 恢复时 ACP 会话 ID 已知：带上它核心才能把 MCP 调用认到链上（ADR-0061 第 10 节）。
            {"name": "wonton-session", "value": "session-1"}
        ])
    );
    drop(client);
    agent.finish().await;
}

#[tokio::test]
async fn oversized_agent_line_disconnects_and_fails_the_pending_prompt() {
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
    let prompted = tokio::spawn({
        let client = client.clone();
        async move {
            bounded(
                client.prompt(
                    "session-1",
                    vec![json!({"type": "text", "text": "hi"})],
                    None,
                    1024,
                    Duration::from_secs(2),
                    zh(),
                ),
                "prompt",
            )
            .await
        }
    });
    bounded(
        async {
            while agent.pending_count() == 0 {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        },
        "等挂起的轮次",
    )
    .await;
    // 超过上限（4 MiB）不带换行的一行，后面跟换行和一条正常帧：桥读到上限就断开，
    // 后面的帧不再读。断开后写端只多占内核缓冲，写得进去。
    agent.send_raw(&vec![b'x'; MAX_FRAME_BYTES + 1]).await;
    agent.send_raw(b"\n").await;
    agent
        .tool_update(
            "session-1",
            "tool_call",
            "call-1",
            "edit",
            "/workspace/x.rs",
        )
        .await;
    let mut closed = client.closed();
    assert!(bounded(closed.changed(), "closed").await.is_ok());
    assert!(*closed.borrow());
    match bounded(prompted, "prompt").await.unwrap() {
        Err(Error::TransportUnavailable(detail)) => {
            assert_eq!(detail, Detail::msg("acp-connection-closed"));
        }
        other => panic!("挂起的轮次应该报断开，实际是 {other:?}"),
    }
    drop(client);
    agent.finish().await;
}

/// 底层流替身：每次最多交 [`READ_CHUNK`] 字节，记下累计交出的字节数。累计超过
/// `MAX_FRAME_BYTES + 2 * READ_CHUNK` 直接报错：实现退化成整行缓冲时会撞上。
const READ_CHUNK: usize = 8 * 1024;

struct ChunkedReader {
    data: Vec<u8>,
    pos: usize,
    read: usize,
}

impl ChunkedReader {
    fn new(data: Vec<u8>) -> Self {
        Self {
            data,
            pos: 0,
            read: 0,
        }
    }
}

impl AsyncRead for ChunkedReader {
    fn poll_read(
        self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let this = self.get_mut();
        if this.pos >= this.data.len() {
            return Poll::Ready(Ok(()));
        }
        let end = (this.pos + READ_CHUNK).min(this.data.len());
        if end > MAX_FRAME_BYTES + 2 * READ_CHUNK {
            return Poll::Ready(Err(std::io::Error::other(
                "读进内存的字节超过了上限加两个读块",
            )));
        }
        buf.put_slice(&this.data[this.pos..end]);
        this.read = this.read.max(end);
        this.pos = end;
        Poll::Ready(Ok(()))
    }
}

#[tokio::test]
async fn frame_limit_takes_three_mib_lines_and_rejects_over() {
    // 3 MiB 的合法 JSON 行照常读成一帧：qwen 的技能清单（约 650 KiB）留了余量。
    let payload = {
        let mut body = vec![b'a'; 3 * 1024 * 1024];
        body[0] = b'"';
        let mut line = Vec::with_capacity(body.len() + 16);
        line.extend_from_slice(
            b"{\"jsonrpc\":\"2.0\",\"method\":\"qwen/notify\",\"params\":{\"text\":",
        );
        line.extend_from_slice(&body);
        line.extend_from_slice(b"\"}");
        line
    };
    let mut data = payload.clone();
    data.push(b'\n');
    let mut reader = BufReader::new(ChunkedReader::new(data));
    let mut frame = Vec::new();
    assert_eq!(
        frame::read_frame(&mut reader, &mut frame).await,
        frame::FrameRead::Frame
    );
    assert_eq!(frame, payload);

    // 超过上限（32 MiB）：报超长。
    let mut data = vec![b'x'; MAX_FRAME_BYTES + 1];
    data.push(b'\n');
    let mut reader = BufReader::new(ChunkedReader::new(data));
    let mut frame = Vec::new();
    assert_eq!(
        frame::read_frame(&mut reader, &mut frame).await,
        frame::FrameRead::Oversize
    );
}

#[tokio::test]
async fn oversized_line_stops_at_the_limit_without_reading_the_next_frame() {
    // 超过上限（32 MiB）不带换行，之后才是换行和一条正常帧：读到上限就得停，后面的字节不进内存。
    let mut data = vec![b'x'; MAX_FRAME_BYTES + 1];
    data.extend_from_slice(b"\n{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{}}\n");
    let mut reader = BufReader::new(ChunkedReader::new(data));
    let mut frame = Vec::new();
    assert_eq!(
        frame::read_frame(&mut reader, &mut frame).await,
        frame::FrameRead::Oversize
    );
    let traffic = reader.get_ref().read;
    assert!(
        traffic <= MAX_FRAME_BYTES + 1 + READ_CHUNK,
        "从底层读了 {traffic} 字节，超过上限加一个读块"
    );
    assert!(frame.len() <= MAX_FRAME_BYTES + 1);
}

#[tokio::test]
async fn frame_limit_takes_lines_up_to_the_limit_and_rejects_over_by_one() {
    // 正好上限：照常一帧。
    let mut data = vec![b'x'; MAX_FRAME_BYTES];
    data.push(b'\n');
    let mut reader = BufReader::new(ChunkedReader::new(data));
    let mut frame = Vec::new();
    assert_eq!(
        frame::read_frame(&mut reader, &mut frame).await,
        frame::FrameRead::Frame
    );
    assert_eq!(frame.len(), MAX_FRAME_BYTES);

    // 上限加一：超长。
    let mut data = vec![b'x'; MAX_FRAME_BYTES + 1];
    data.push(b'\n');
    let mut reader = BufReader::new(ChunkedReader::new(data));
    let mut frame = Vec::new();
    assert_eq!(
        frame::read_frame(&mut reader, &mut frame).await,
        frame::FrameRead::Oversize
    );

    // 内容正好上限、以 \r\n 结尾：去回车后还是一帧。
    let mut data = vec![b'x'; MAX_FRAME_BYTES];
    data.extend_from_slice(b"\r\n");
    let mut reader = BufReader::new(ChunkedReader::new(data));
    let mut frame = Vec::new();
    assert_eq!(
        frame::read_frame(&mut reader, &mut frame).await,
        frame::FrameRead::Frame
    );
    assert_eq!(frame.len(), MAX_FRAME_BYTES);
}

#[tokio::test]
async fn restored_sessions_get_the_mode_new_set_again() {
    let agent = FakeAgent::start(Scenario::Normal).await;
    let client = bounded(AcpClient::connect(&agent.socket, broker()), "initialize")
        .await
        .unwrap();
    bounded(
        client.load_or_resume(
            "session-1",
            "/workspace",
            "http://mcp.wonton.internal/mcp",
            &origin(None),
            "acceptEdits",
        ),
        "session/load",
    )
    .await
    .unwrap();
    let requests = agent.requests().await;
    let set_modes: Vec<_> = requests
        .iter()
        .filter(|request| request["method"] == "session/set_mode")
        .collect();
    assert_eq!(set_modes.len(), 1, "{requests:?}");
    assert_eq!(set_modes[0]["params"]["sessionId"], "session-1");
    assert_eq!(set_modes[0]["params"]["modeId"], "acceptEdits");
    drop(client);
    agent.finish().await;
}

#[tokio::test]
async fn an_expired_permission_cancels_the_session_before_answering_cancelled() {
    let (agent, core, client, prompt) = started_with(json!({"outcome": "expired"})).await;
    let id = agent.ask_permission("session-1", "src/a.rs").await;
    let responses = permission_response(&agent, &id).await;
    assert_eq!(responses.len(), 1);
    assert_eq!(responses[0]["result"]["outcome"]["outcome"], "cancelled");

    // 过期的卡片等不到答案：先发 `session/cancel`，假 Agent 照实撤了这一轮，然后才
    // 收到这次请求的 `cancelled`（ADR-0061）。
    let incoming = agent.incoming().await;
    let cancel_at = incoming
        .iter()
        .position(|message| {
            message["method"] == "session/cancel" && message["params"]["sessionId"] == "session-1"
        })
        .expect("桥没有发 session/cancel");
    let answer_at = incoming
        .iter()
        .position(|message| message.get("method").is_none() && message["id"] == id)
        .expect("桥没有回答权限请求");
    assert!(cancel_at < answer_at, "要先 session/cancel 再回 cancelled");

    let result = bounded(prompt, "prompt")
        .await
        .expect("prompt 任务 panic")
        .expect("prompt 应答");
    assert_eq!(result.stop_reason, "cancelled");
    core.finish();
    drop(client);
    agent.finish().await;
}

#[tokio::test]
async fn agents_without_modes_tolerate_the_restore_set_mode() {
    let agent = FakeAgent::start(Scenario::NoModes).await;
    let client = bounded(AcpClient::connect(&agent.socket, broker()), "initialize")
        .await
        .unwrap();
    bounded(
        client.load_or_resume(
            "session-1",
            "/workspace",
            "http://mcp.wonton.internal/mcp",
            &origin(None),
            "plan",
        ),
        "session/load",
    )
    .await
    .unwrap();
    // set_mode 发了但 Agent 回 Method not found：照 new 的路子跳过，恢复照常成功。
    let requests = agent.requests().await;
    assert!(
        requests
            .iter()
            .any(|request| request["method"] == "session/set_mode"),
        "{requests:?}"
    );
    drop(client);
    agent.finish().await;
}

#[tokio::test]
async fn a_cancelled_permission_does_not_cancel_the_session() {
    let (agent, core, client, prompt) = started_with(json!({"outcome": "cancelled"})).await;
    let id = agent.ask_permission("session-1", "src/a.rs").await;
    let responses = permission_response(&agent, &id).await;
    assert_eq!(responses.len(), 1);
    assert_eq!(responses[0]["result"]["outcome"]["outcome"], "cancelled");

    // 卡片被撤回只答复这一次请求，会话照常跑完这一轮。
    let incoming = agent.incoming().await;
    assert!(
        !incoming
            .iter()
            .any(|message| message["method"] == "session/cancel"),
        "撤回不该取消会话"
    );

    let session = agent.respond_prompt(0, "done").await;
    assert_eq!(session, "session-1");
    let result = bounded(prompt, "prompt")
        .await
        .expect("prompt 任务 panic")
        .expect("prompt 应答");
    assert_eq!(result.stop_reason, "end_turn");
    assert_eq!(result.text, "done");
    core.finish();
    drop(client);
    agent.finish().await;
}
