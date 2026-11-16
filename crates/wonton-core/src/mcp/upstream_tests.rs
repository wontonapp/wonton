//! 总线连插件的连接：超限和失败按请求归属，插件的错误不拆共享的连接，只丢出错的那条连接，卡死的会话会被换掉。
//! 假插件在 [`super::upstream_support`]。

use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use super::test_support::{Notes, serve_notes};
use super::upstream::{CallError, ReadError, UncertainKind, Upstreams};
use super::upstream_support::*;
use crate::plugin::{McpServer, PluginId, ToolLimits};

/// 回归时测试要失败，不能卡住。
const WITHIN: Duration = Duration::from_secs(20);

async fn within<T>(future: impl Future<Output = T>) -> T {
    tokio::time::timeout(WITHIN, future)
        .await
        .expect("到期了还没有结果")
}

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

/// 超过上限的结果，失败说明里带上限和至少读到了多少：Agent 才知道插件做到了哪一步、结果有多大。
#[tokio::test(flavor = "multi_thread")]
async fn an_oversized_result_reports_the_limit_and_the_bytes_read() {
    for content_type in ["application/json", "text/event-stream"] {
        let upstreams = Upstreams::with_limits(1 << 20, Duration::from_secs(5));
        let plugin = plugin(content_type).await;
        let server = &plugin.server;
        let error = upstreams
            .call(server, "big", NO_ARGUMENTS, std::future::pending())
            .await
            .expect_err("超过上限要报出来");
        let CallError::Uncertain { kind, message } = error else {
            panic!("{content_type}: 应该是结果不确定：{error:?}");
        };
        assert_eq!(kind, UncertainKind::ResultTooLarge, "{message}");
        assert!(
            message.contains("上限 1.0 MiB"),
            "{content_type}: {message}"
        );
        // 读到超过上限才停（传输层按收到的字节数记，块的大小看连接分片），所以总有一条“至少读了”。
        assert!(message.contains("至少读了"), "{content_type}: {message}");
        plugin.stop().await;
    }
}

/// 插件声明的大小超过上限时，一个字节没读也说清声明了多少，不算成读到了多少。
#[tokio::test(flavor = "multi_thread")]
async fn a_result_that_declares_too_much_is_reported_before_reading() {
    let upstreams = Upstreams::with_limits(1 << 20, Duration::from_secs(5));
    let server = server_at(declared_plugin().await);
    let error = upstreams
        .call(&server, "declared", NO_ARGUMENTS, std::future::pending())
        .await
        .expect_err("超过上限要报出来");
    let CallError::Uncertain { kind, message } = error else {
        panic!("应该是结果不确定：{error:?}");
    };
    assert_eq!(kind, UncertainKind::ResultTooLarge, "{message}");
    assert!(message.contains("上限 1.0 MiB"), "{message}");
    assert!(message.contains("插件声明有 100.0 MiB"), "{message}");
    assert!(!message.contains("读了"), "{message}");
}

/// 核心和 Bot 共用一个客户端：超限只算在超限的那个请求头上，另一个照常拿到结果。
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
        headers: Default::default(),
        tool_limits: Default::default(),
        tools: None,
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
