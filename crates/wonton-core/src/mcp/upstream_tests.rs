//! 总线连插件的连接：超限和失败按请求归属，插件的错误不拆共享的连接，只丢出错的那条连接，卡死的会话会被换掉。

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use axum::response::{IntoResponse, Response};
use rmcp::model::JsonObject;
use serde_json::{Value, json};

use super::test_support::{Notes, serve_notes};
use super::upstream::{CallError, ReadError, UncertainKind, Upstreams};
use crate::plugin::{McpServer, PluginId, ToolLimits};

const SLOW: Duration = Duration::from_millis(200);
/// 回归时测试要失败，不能卡住。
const WITHIN: Duration = Duration::from_secs(20);

async fn within<T>(future: impl Future<Output = T>) -> T {
    tokio::time::timeout(WITHIN, future)
        .await
        .expect("到期了还没有结果")
}

fn result(id: &Value, result: Value) -> Response {
    axum::Json(json!({"jsonrpc": "2.0", "id": id, "result": result})).into_response()
}

fn endless(content_type: &'static str) -> Response {
    let chunks = futures_util::stream::repeat_with(|| {
        Ok::<_, std::convert::Infallible>(axum::body::Bytes::from(vec![b'x'; 1 << 16]))
    });
    let body = axum::body::Body::from_stream(chunks);
    ([(hyper::header::CONTENT_TYPE, content_type)], body).into_response()
}

/// 响应体写到一半连接断了。
fn cut(content_type: &'static str) -> Response {
    let chunks = futures_util::stream::iter([
        Ok(axum::body::Bytes::from_static(b"data: {\"jsonrpc")),
        Err(std::io::Error::other("断了")),
    ]);
    let body = axum::body::Body::from_stream(chunks);
    ([(hyper::header::CONTENT_TYPE, content_type)], body).into_response()
}

fn text(uri: &str) -> Value {
    json!({"contents": [{"uri": uri, "text": "好"}]})
}

fn tool_text() -> Value {
    json!({"content": [{"type": "text", "text": "好"}]})
}

async fn read(
    id: &Value,
    uri: &str,
    content_type: &'static str,
    held: &tokio::sync::Notify,
) -> Response {
    match uri {
        "x://big" => endless(content_type),
        "x://refused" => axum::Json(json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": {"code": -32002, "message": "没有这个资源"},
        }))
        .into_response(),
        "x://http500" => hyper::StatusCode::INTERNAL_SERVER_ERROR.into_response(),
        "x://wordy500" => (
            hyper::StatusCode::INTERNAL_SERVER_ERROR,
            "磁盘坏了".repeat(1 << 16),
        )
            .into_response(),
        "x://huge500" => {
            let mut response = endless("text/plain");
            *response.status_mut() = hyper::StatusCode::INTERNAL_SERVER_ERROR;
            response
        }
        // supergateway 换会话后这样回。
        "x://null-id" => (
            hyper::StatusCode::INTERNAL_SERVER_ERROR,
            axum::Json(json!({"jsonrpc": "2.0", "id": null, "error": {"code": -32000, "message": "会话没了"}})),
        )
            .into_response(),
        "x://wrong-error" => axum::Json(json!({
            "jsonrpc": "2.0",
            "id": 987654,
            "error": {"code": -32000, "message": "不是你的"},
        }))
        .into_response(),
        "x://wrong-result" => result(&json!(987654), text(uri)),
        "x://html" => ([(hyper::header::CONTENT_TYPE, "text/html")], "<p>登录</p>").into_response(),
        "x://garbled" => (
            [(hyper::header::CONTENT_TYPE, "application/json")],
            "{ 不是",
        )
            .into_response(),
        "x://slowcut" => {
            tokio::time::sleep(SLOW).await;
            cut(content_type)
        }
        "x://cut" => cut(content_type),
        "x://slow" => {
            tokio::time::sleep(SLOW).await;
            result(id, text(uri))
        }
        "x://held" => {
            held.notified().await;
            result(id, text(uri))
        }
        _ => result(id, text(uri)),
    }
}

/// `tools/list` 怎么回。
#[derive(Clone, Copy, Default)]
enum Lists {
    #[default]
    Empty,
    /// 每一页都不大，加起来超过上限。
    Heavy,
    /// 一直有下一页。
    Endless,
    /// 下一页的 cursor 总是同一个。
    SameCursor,
    /// 一页就有太多工具。
    TooMany,
}

fn tool_named(name: String) -> Value {
    json!({"name": name, "inputSchema": {"type": "object"}})
}

fn list(id: &Value, lists: Lists, cursor: &Value) -> Response {
    let tools = || vec![tool_named("a".to_owned())];
    match lists {
        Lists::Empty => result(id, json!({"tools": []})),
        Lists::Endless => {
            let next = cursor.as_str().unwrap_or_default().len() + 1;
            result(
                id,
                json!({"tools": tools(), "nextCursor": "x".repeat(next)}),
            )
        }
        Lists::SameCursor => result(id, json!({"tools": tools(), "nextCursor": "again"})),
        Lists::Heavy => {
            let next = cursor.as_str().unwrap_or_default().len() + 1;
            let mut heavy = tool_named("heavy".to_owned());
            heavy["description"] = json!("x".repeat(1 << 20));
            result(
                id,
                json!({"tools": [heavy], "nextCursor": "x".repeat(next)}),
            )
        }
        Lists::TooMany => {
            let tools: Vec<_> = (0..3_000).map(|n| tool_named(n.to_string())).collect();
            result(id, json!({"tools": tools}))
        }
    }
}

/// 进度事件，最后是结果。`steps` 为空时一直发进度。
fn with_progress(id: &Value, token: Value, steps: Option<usize>) -> Response {
    let event = |message: Value| {
        Ok::<_, std::convert::Infallible>(axum::body::Bytes::from(format!("data: {message}\n\n")))
    };
    let progress = {
        let token = token.clone();
        move |step: usize| {
            json!({
                "jsonrpc": "2.0",
                "method": "notifications/progress",
                "params": {"progressToken": token.clone(), "progress": step},
            })
        }
    };
    let id = id.clone();
    let chunks = futures_util::stream::unfold(0usize, move |step| {
        let (progress, id) = (progress.clone(), id.clone());
        async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            if steps.is_some_and(|steps| step > steps) {
                return None;
            }
            let message = if steps == Some(step) {
                json!({"jsonrpc": "2.0", "id": id, "result": tool_text()})
            } else {
                progress(step)
            };
            Some((event(message), step + 1))
        }
    });
    let body = axum::body::Body::from_stream(chunks);
    ([(hyper::header::CONTENT_TYPE, "text/event-stream")], body).into_response()
}

async fn answer(state: Scripted, content_type: &'static str, body: String) -> Response {
    let message: Value = serde_json::from_str(&body).unwrap();
    let id = if state.string_ids.load(Ordering::SeqCst) {
        &json!(message["id"].to_string())
    } else {
        &message["id"]
    };
    let params = &message["params"];
    match message["method"].as_str().unwrap_or_default() {
        "initialize" => {
            state.initializes.fetch_add(1, Ordering::SeqCst);
            result(
                id,
                json!({
                    "protocolVersion": "2025-11-25",
                    "capabilities": {"resources": {}, "tools": {}},
                    "serverInfo": {"name": "scripted", "version": "1"},
                }),
            )
        }
        "resources/read" => {
            let uri = params["uri"].as_str().unwrap();
            read(id, uri, content_type, &state.held_reads).await
        }
        "tools/call" if params["name"] == "big" => endless(content_type),
        "tools/call" if params["name"] == "silent" => std::future::pending().await,
        "tools/call" if params["name"] == "progress" || params["name"] == "chatty" => {
            let steps = (params["name"] == "progress").then_some(8);
            with_progress(id, params["_meta"]["progressToken"].clone(), steps)
        }
        "tools/call" => {
            tokio::time::sleep(SLOW).await;
            result(id, tool_text())
        }
        "tools/list" => {
            // 还有几次不回答：会话卡死的样子，TCP 没断。
            let left = state
                .stuck_lists
                .try_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1));
            if left.is_ok() {
                std::future::pending::<()>().await;
            }
            let lists = *state.lists.lock().unwrap();
            list(id, lists, &params["cursor"])
        }
        "notifications/cancelled" => {
            state.cancelled.fetch_add(1, Ordering::SeqCst);
            hyper::StatusCode::ACCEPTED.into_response()
        }
        _ => hyper::StatusCode::ACCEPTED.into_response(),
    }
}

/// 假插件的状态。
#[derive(Clone, Default)]
struct Scripted {
    initializes: Arc<AtomicUsize>,
    stuck_lists: Arc<AtomicUsize>,
    /// 收到了几次 `notifications/cancelled`。
    cancelled: Arc<AtomicUsize>,
    lists: Arc<std::sync::Mutex<Lists>>,
    /// 把数字 id 回成字符串。
    string_ids: Arc<AtomicBool>,
    /// `x://held` 等到这里放行才回答。
    held_reads: Arc<tokio::sync::Notify>,
}

/// 假插件，按资源的地址或工具的名字决定怎么回。超限和断开的响应用 `content_type`，其余都是普通 JSON。
struct Plugin {
    server: McpServer,
    state: Scripted,
    initializes: Arc<AtomicUsize>,
    stuck_lists: Arc<AtomicUsize>,
    stop: tokio::sync::oneshot::Sender<()>,
    serving: tokio::task::JoinHandle<()>,
}

impl Plugin {
    /// 监听停了，空闲的连接也关了：和容器重启一样。
    async fn stop(self) -> McpServer {
        self.stop.send(()).unwrap();
        self.serving.await.unwrap();
        self.server
    }
}

async fn plugin(content_type: &'static str) -> Plugin {
    let state = Scripted::default();
    let served = state.clone();
    let app = axum::Router::new().route(
        "/mcp",
        axum::routing::post(move |body: String| answer(served.clone(), content_type, body)),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (stop, stopped) = tokio::sync::oneshot::channel();
    let serving = tokio::spawn(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(async { drop(stopped.await) })
            .await
            .unwrap();
    });
    Plugin {
        stop,
        serving,
        server: server_at(format!("http://{address}/mcp")),
        initializes: state.initializes.clone(),
        stuck_lists: state.stuck_lists.clone(),
        state,
    }
}

fn server_at(url: String) -> McpServer {
    McpServer {
        id: PluginId::parse("app.example.notes").unwrap(),
        title: Default::default(),
        url,
        socket: None,
        tool_limits: Default::default(),
    }
}

const NO_ARGUMENTS: Option<JsonObject> = None;

#[tokio::test(flavor = "multi_thread")]
async fn a_huge_resource_is_stopped_while_reading_and_reported_as_too_large() {
    for content_type in ["application/json", "text/event-stream"] {
        let upstreams = Upstreams::with_limits(1 << 20, Duration::from_secs(5));
        let plugin = plugin(content_type).await;
        let read = upstreams.read_resource(&plugin.server, "x://big", 1 << 20);
        let result = tokio::time::timeout(Duration::from_secs(30), read)
            .await
            .expect("上限要在读响应体时生效");
        assert!(
            matches!(result, Err(ReadError::TooLarge)),
            "{content_type}: {result:?}"
        );
    }
}

/// 核心和 Bot 共用连接：超限只算在超限的那个请求头上，另一个照常拿到结果，连接也不用重建。
#[tokio::test(flavor = "multi_thread")]
async fn an_oversized_response_is_charged_to_its_own_request_only() {
    for content_type in ["application/json", "text/event-stream"] {
        let upstreams = Upstreams::with_limits(1 << 20, Duration::from_secs(5));
        let plugin = plugin(content_type).await;
        let server = &plugin.server;
        for round in 0..10 {
            let (slow, big) = tokio::join!(
                upstreams.call(server, "slow", NO_ARGUMENTS, std::future::pending()),
                upstreams.read_resource(server, "x://big", 1 << 20),
            );
            assert!(slow.is_ok(), "{content_type} 第 {round} 轮：{slow:?}");
            assert!(
                matches!(big, Err(ReadError::TooLarge)),
                "{content_type} 第 {round} 轮：{big:?}"
            );

            let (big, slow) = tokio::join!(
                upstreams.call(server, "big", NO_ARGUMENTS, std::future::pending()),
                upstreams.read_resource(server, "x://slow", 64),
            );
            assert!(big.is_err(), "{content_type} 第 {round} 轮");
            assert!(slow.is_ok(), "{content_type} 第 {round} 轮：{slow:?}");
        }
        assert_eq!(
            plugin.initializes.load(Ordering::SeqCst),
            1,
            "{content_type}"
        );
    }
}

#[tokio::test]
async fn an_error_from_the_plugin_leaves_the_shared_connection_alone() {
    let upstreams = Upstreams::new();
    let plugin = plugin("application/json").await;
    let server = &plugin.server;
    let (slow, refused) = tokio::join!(
        upstreams.call(server, "slow", NO_ARGUMENTS, std::future::pending()),
        upstreams.read_resource(server, "x://refused", 64),
    );
    assert!(slow.is_ok(), "{slow:?}");
    assert!(
        matches!(refused, Err(ReadError::Upstream(_))),
        "{refused:?}"
    );
    assert_eq!(plugin.initializes.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn a_read_that_times_out_leaves_the_shared_connection_alone() {
    let upstreams = Upstreams::with_read_timeout(SLOW / 4);
    let plugin = plugin("application/json").await;
    let server = &plugin.server;
    let (slow, timed_out) = tokio::join!(
        upstreams.call(server, "slow", NO_ARGUMENTS, std::future::pending()),
        upstreams.read_resource(server, "x://slow", 64),
    );
    assert!(slow.is_ok(), "{slow:?}");
    assert!(
        matches!(timed_out, Err(ReadError::Unreachable(_))),
        "{timed_out:?}"
    );
    assert_eq!(plugin.initializes.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn a_transport_failure_reads_as_unreachable_and_the_next_call_reconnects() {
    for content_type in ["application/json", "text/event-stream"] {
        let upstreams = Upstreams::new();
        let plugin = plugin(content_type).await;
        let server = &plugin.server;
        let failed = upstreams.read_resource(server, "x://cut", 64).await;
        assert!(
            matches!(failed, Err(ReadError::Unreachable(_))),
            "{content_type}: {failed:?}"
        );
        assert!(upstreams.read_resource(server, "x://ok", 64).await.is_ok());
        assert_eq!(
            plugin.initializes.load(Ordering::SeqCst),
            2,
            "{content_type}"
        );
    }
}

/// 插件在，只是这一个请求的回答不对：和内网地址回 500 一样是插件的错，连接照用，并发的请求也不受影响。
#[tokio::test]
async fn an_http_error_for_one_request_is_the_plugins_error_and_keeps_the_connection() {
    let upstreams = Upstreams::with_limits(1 << 20, Duration::from_secs(5));
    let plugin = plugin("application/json").await;
    let server = &plugin.server;
    for uri in [
        "x://null-id",
        "x://wrong-error",
        "x://wrong-result",
        "x://http500",
        "x://wordy500",
        "x://huge500",
        "x://html",
        "x://garbled",
    ] {
        let (slow, failed) = tokio::join!(
            upstreams.call(server, "slow", NO_ARGUMENTS, std::future::pending()),
            upstreams.read_resource(server, uri, 64),
        );
        assert!(slow.is_ok(), "{uri}: {slow:?}");
        let Err(ReadError::Upstream(why)) = failed else {
            panic!("{uri}: {failed:?}");
        };
        assert!(why.len() < 4 << 10, "{uri}：错误消息有 {} 字节", why.len());
    }
    assert_eq!(plugin.initializes.load(Ordering::SeqCst), 1);
}

/// 旧连接上的调用晚一点才失败，不能把别的调用刚建好的新连接也丢掉。
#[tokio::test]
async fn a_late_failure_on_the_old_connection_keeps_the_new_one() {
    let upstreams = Upstreams::new();
    let plugin = plugin("application/json").await;
    let server = &plugin.server;
    assert!(upstreams.read_resource(server, "x://ok", 64).await.is_ok());
    let late = upstreams.read_resource(server, "x://slowcut", 64);
    let reconnect = async {
        tokio::task::yield_now().await;
        let failed = upstreams.read_resource(server, "x://cut", 64).await;
        assert!(
            matches!(failed, Err(ReadError::Unreachable(_))),
            "{failed:?}"
        );
        assert!(upstreams.read_resource(server, "x://ok", 64).await.is_ok());
    };
    let (late, ()) = tokio::join!(late, reconnect);
    assert!(matches!(late, Err(ReadError::Unreachable(_))), "{late:?}");
    assert!(upstreams.read_resource(server, "x://ok", 64).await.is_ok());
    assert_eq!(plugin.initializes.load(Ordering::SeqCst), 2);
}

/// 期限短到连握手也包进去的话，CI 负载一高就失败：先连好，再缩短。
async fn connected(plugin: &Plugin) -> Upstreams {
    let mut upstreams = Upstreams::new();
    upstreams.tools(&plugin.server).await.unwrap();
    upstreams.set_tools_timeout(Duration::from_millis(50));
    upstreams
}

#[tokio::test]
async fn a_session_that_keeps_timing_out_is_replaced() {
    let plugin = plugin("application/json").await;
    let server = &plugin.server;
    let mut upstreams = connected(&plugin).await;
    plugin.stuck_lists.store(3, Ordering::SeqCst);
    for _ in 0..3 {
        assert!(upstreams.tools(server).await.is_err());
    }
    assert_eq!(plugin.initializes.load(Ordering::SeqCst), 1);
    upstreams.set_tools_timeout(Duration::from_secs(5));
    assert!(upstreams.tools(server).await.is_ok());
    assert_eq!(plugin.initializes.load(Ordering::SeqCst), 2);
}

/// 只是偶尔慢：中间拿到过回答，就不算连续。
#[tokio::test]
async fn an_answer_between_timeouts_keeps_the_session() {
    let plugin = plugin("application/json").await;
    let server = &plugin.server;
    let mut upstreams = connected(&plugin).await;
    for _ in 0..3 {
        plugin.stuck_lists.store(2, Ordering::SeqCst);
        assert!(upstreams.tools(server).await.is_err());
        assert!(upstreams.tools(server).await.is_err());
        upstreams.set_tools_timeout(Duration::from_secs(5));
        assert!(upstreams.tools(server).await.is_ok());
        upstreams.set_tools_timeout(Duration::from_millis(50));
    }
    assert_eq!(plugin.initializes.load(Ordering::SeqCst), 1);
}

/// 几个 Agent 在同一次慢的时候一起超时，算一次。
#[tokio::test]
async fn timeouts_at_the_same_moment_count_once() {
    let plugin = plugin("application/json").await;
    let server = &plugin.server;
    let mut upstreams = connected(&plugin).await;
    plugin.stuck_lists.store(4, Ordering::SeqCst);
    let (a, b, c, d) = tokio::join!(
        upstreams.tools(server),
        upstreams.tools(server),
        upstreams.tools(server),
        upstreams.tools(server),
    );
    assert!(a.is_err() && b.is_err() && c.is_err() && d.is_err());
    upstreams.set_tools_timeout(Duration::from_secs(5));
    assert!(upstreams.tools(server).await.is_ok());
    assert_eq!(plugin.initializes.load(Ordering::SeqCst), 1);
}

/// 串行的服务器做长调用时 `tools/list` 排在后面：这不是卡死，会话和调用都要留着。
#[tokio::test]
async fn a_long_call_keeps_the_session_while_lists_time_out() {
    let plugin = plugin("application/json").await;
    let server = &plugin.server;
    let upstreams = connected(&plugin).await;
    plugin.stuck_lists.store(4, Ordering::SeqCst);
    let call = upstreams.call(server, "progress", NO_ARGUMENTS, std::future::pending());
    let lists = async {
        tokio::task::yield_now().await;
        for _ in 0..4 {
            assert!(upstreams.tools(server).await.is_err());
        }
    };
    let (call, ()) = within(async { tokio::join!(call, lists) }).await;
    assert!(call.is_ok(), "{call:?}");
    assert_eq!(plugin.initializes.load(Ordering::SeqCst), 1);
}

/// 长时间的读也一样：有状态的子进程不能因为它排在后面的 `tools/list` 被丢掉。
#[tokio::test]
async fn a_long_read_keeps_the_session_while_lists_time_out() {
    let plugin = plugin("application/json").await;
    let server = &plugin.server;
    let upstreams = connected(&plugin).await;
    plugin.stuck_lists.store(3, Ordering::SeqCst);
    let read = upstreams.read_resource(server, "x://held", 64);
    let lists = async {
        tokio::task::yield_now().await;
        for _ in 0..3 {
            assert!(upstreams.tools(server).await.is_err());
        }
        plugin.state.held_reads.notify_one();
    };
    let (read, ()) = within(async { tokio::join!(read, lists) }).await;
    assert!(read.is_ok(), "{read:?}");
    assert_eq!(plugin.initializes.load(Ordering::SeqCst), 1);
}

/// 插件重启了，缓存的连接还没发现：请求没到插件，算失败，不算不确定，Agent 可以再试。
#[tokio::test]
async fn a_call_that_cannot_even_connect_is_not_sent_rather_than_uncertain() {
    let plugin = plugin("application/json").await;
    let upstreams = Upstreams::new();
    upstreams.tools(&plugin.server).await.unwrap();
    let server = plugin.stop().await;
    let called =
        within(upstreams.call(&server, "tool", NO_ARGUMENTS, std::future::pending())).await;
    assert!(matches!(called, Err(CallError::NotSent(_))), "{called:?}");
}

/// 有的服务把数字 id 回成字符串：rmcp 接受，总线也要接受，否则握手就失败。
#[tokio::test]
async fn an_id_that_comes_back_as_a_string_still_answers() {
    let plugin = plugin("application/json").await;
    plugin.state.string_ids.store(true, Ordering::SeqCst);
    let upstreams = Upstreams::new();
    upstreams.tools(&plugin.server).await.unwrap();
    assert!(
        upstreams
            .read_resource(&plugin.server, "x://ok", 64)
            .await
            .is_ok()
    );
}

/// 换掉会话时，上面还在跑的调用结束，原因写明，不混进插件的错误。
#[tokio::test]
async fn replacing_a_session_ends_what_was_running_on_it() {
    let plugin = plugin("application/json").await;
    let server = &plugin.server;
    let upstreams = connected(&plugin).await;
    plugin.stuck_lists.store(3, Ordering::SeqCst);
    for _ in 0..2 {
        assert!(upstreams.tools(server).await.is_err());
    }
    // 这个调用比第三次列工具晚开始，解释不了它的超时。
    let (listed, called) = tokio::join!(upstreams.tools(server), async {
        tokio::time::sleep(Duration::from_millis(10)).await;
        within(upstreams.call(server, "silent", NO_ARGUMENTS, std::future::pending())).await
    });
    assert!(listed.is_err());
    let Err(CallError::Uncertain { kind, message }) = called else {
        panic!("{called:?}");
    };
    assert_eq!(kind, UncertainKind::SessionReplaced);
    assert!(message.contains("已被换掉"), "{message}");
}

fn limited(plugin: &Plugin, idle: Duration, total: Duration) -> McpServer {
    McpServer {
        tool_limits: ToolLimits { idle, total },
        ..plugin.server.clone()
    }
}

/// 没有进度到期：告诉插件取消，结果算不确定，插件可能已经做了。到期的是空闲期限：只有它不同，等的时间就不同。
#[tokio::test]
async fn a_silent_tool_times_out_as_uncertain_and_the_plugin_is_told() {
    let upstreams = Upstreams::new();
    let plugin = plugin("application/json").await;
    let total = Duration::from_secs(10);
    for (told, idle) in [(1, Duration::from_millis(100)), (2, Duration::from_secs(1))] {
        let server = limited(&plugin, idle, total);
        let started = Instant::now();
        let called = upstreams
            .call(&server, "silent", NO_ARGUMENTS, std::future::pending())
            .await;
        let waited = started.elapsed();
        assert!(waited >= idle && waited < total / 2, "{waited:?}");
        let Err(CallError::Uncertain { kind, message }) = called else {
            panic!("{called:?}");
        };
        assert_eq!(kind, UncertainKind::TimedOut);
        assert!(message.contains("是否已执行不确定"), "{message}");
        within(async {
            while plugin.state.cancelled.load(Ordering::SeqCst) < told {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await;
    }
}

/// 有进度就接着等，直到总期限。
#[tokio::test]
async fn progress_keeps_a_call_alive_up_to_the_total_limit() {
    let upstreams = Upstreams::new();
    let plugin = plugin("application/json").await;
    // 进度每 50 毫秒一次，结果在 400 毫秒之后：比空闲期限长得多。
    let server = limited(&plugin, Duration::from_millis(200), Duration::from_secs(30));
    let called =
        within(upstreams.call(&server, "progress", NO_ARGUMENTS, std::future::pending())).await;
    assert!(called.is_ok(), "{called:?}");

    let server = limited(&plugin, Duration::from_secs(1), Duration::from_millis(300));
    let called =
        within(upstreams.call(&server, "chatty", NO_ARGUMENTS, std::future::pending())).await;
    assert!(
        matches!(
            called,
            Err(CallError::Uncertain {
                kind: UncertainKind::TimedOut,
                ..
            })
        ),
        "{called:?}"
    );
}

#[tokio::test]
async fn a_cancelled_call_tells_the_plugin() {
    let upstreams = Upstreams::new();
    let plugin = plugin("application/json").await;
    let called = upstreams
        .call(
            &plugin.server,
            "silent",
            NO_ARGUMENTS,
            tokio::time::sleep(Duration::from_millis(50)),
        )
        .await;
    assert!(matches!(called, Err(CallError::Cancelled)), "{called:?}");
    assert_eq!(plugin.state.cancelled.load(Ordering::SeqCst), 1);
}

/// 工具列表只有时间限制的话，一个插件就能让核心用完内存：每个 Agent 都会并发地列所有插件。
#[tokio::test]
async fn a_tool_list_that_never_ends_is_cut_off() {
    let plugin = plugin("application/json").await;
    let server = &plugin.server;
    let upstreams = Upstreams::new();
    for (lists, expected) in [
        (Lists::Endless, "超过 50 页"),
        (Lists::Heavy, "超过上限"),
        (Lists::SameCursor, "同一个 cursor"),
        (Lists::TooMany, "超过上限"),
    ] {
        *plugin.state.lists.lock().unwrap() = lists;
        let why = upstreams.tools(server).await.unwrap_err();
        assert!(why.contains(expected), "{why}");
        assert_eq!(upstreams.tools_problem(&server.id), Some(why));
    }
    *plugin.state.lists.lock().unwrap() = Lists::Empty;
    assert!(upstreams.tools(server).await.is_ok());
    assert_eq!(upstreams.tools_problem(&server.id), None);
    assert_eq!(
        plugin.initializes.load(Ordering::SeqCst),
        1,
        "插件的错，不拆连接"
    );
}

/// 只接受连接、从不回答的插件。
async fn stuck_plugin() -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let mut accepted = Vec::new();
        while let Ok((socket, _)) = listener.accept().await {
            accepted.push(socket);
        }
    });
    format!("http://{address}/mcp")
}

#[tokio::test]
async fn a_stuck_plugin_holds_up_nobody_and_times_out() {
    let upstreams = Upstreams::with_limits(1 << 20, Duration::from_secs(2));
    let stuck = McpServer {
        id: PluginId::parse("app.example.stuck").unwrap(),
        title: Default::default(),
        url: stuck_plugin().await,
        socket: None,
        tool_limits: Default::default(),
    };
    let fine = server_at(serve_notes(Notes::default()).await);
    let stuck_read = upstreams.read_resource(&stuck, "x://a", 64);
    let fine_read = async {
        tokio::task::yield_now().await;
        tokio::time::timeout(
            Duration::from_secs(1),
            upstreams.read_resource(&fine, "notes://text", 64),
        )
        .await
        .expect("别的插件卡在握手上，不该挡住这个插件")
    };
    let (stuck_result, fine_result) = tokio::join!(stuck_read, fine_read);
    assert!(fine_result.is_ok(), "{fine_result:?}");
    assert!(
        matches!(stuck_result, Err(ReadError::Unreachable(_))),
        "{stuck_result:?}"
    );
}
