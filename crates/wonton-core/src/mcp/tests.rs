//! 总线的行为：工具汇总、只读工具直接执行、会改数据的工具和装卸插件要主人确认、拒绝和过期都不执行。插件的 MCP
//! 服务和卡片通道用 [`test_support`] 里的替身；装卸插件用假 Docker。经网关的 HTTP 入口见 `gateway::mcp_tests`。

use std::fs;
use std::sync::atomic::Ordering;
use std::time::Duration;

use rmcp::model::{CallToolRequestParams, CallToolResult};
use serde_json::{Value, json};
use wonton_protocol::{Answer, CardChoice, ConfirmationState};

use super::AnswerError;
use super::cards::Draft;
use super::confirm::DEFAULT_WINDOW;
use super::test_support::{Notes, serve_notes};
use super::upstream::{ReadError, Upstreams};
use crate::plugin::docker::Engine;
use crate::plugin::fake_docker::FakeDocker;
use crate::plugin::languages::Languages;
use crate::plugin::{McpServer, PluginId, PluginPackage, PluginStatus};

pub(super) use crate::mcp::test_setup::*;
pub(super) use crate::mcp::tool_failure_tests;

pub(super) fn text(result: &CallToolResult) -> String {
    result
        .content
        .iter()
        .filter_map(|c| c.as_text().map(|t| t.text.clone()))
        .collect()
}

pub(super) fn failed(result: &CallToolResult) -> bool {
    result.is_error == Some(true)
}

#[tokio::test]
async fn agents_see_the_core_tools_and_the_tools_of_running_plugins() {
    let s = setup(Engine::unreachable_for_test(), Duration::from_secs(60)).await;
    let tools = s.bus.tools(&s.caller).await;
    let names: Vec<&str> = tools.iter().map(|t| t.name.as_ref()).collect();
    for expected in [
        "wonton__list_plugins",
        "wonton__search_catalog",
        "wonton__install_plugin",
        "wonton__uninstall_plugin",
        "wonton__help",
        "app-example-notes__search",
        "app-example-notes__delete",
        // 清单没写 `mcp.tools`：全部提供。
        "app-example-notes__find",
        "app-example-notes__archive",
    ] {
        assert!(names.contains(&expected), "{names:?}");
    }
    let read_only = |name: &str| {
        let tool = tools.iter().find(|t| t.name == name).unwrap();
        tool.annotations.as_ref().and_then(|a| a.read_only_hint)
    };
    assert_eq!(
        read_only("app-example-notes__search"),
        Some(true),
        "插件的注解原样带过来"
    );
    assert_eq!(read_only("wonton__install_plugin"), Some(false));
}

/// 清单写了 `mcp.tools` 就只提供列表里的工具：其余的不出现在总线上，调用它们和调用不存在的工具一样被拒，
/// 请求不发给插件。
#[tokio::test]
async fn a_plugin_only_provides_the_tools_its_manifest_lists() {
    let s = setup_with(
        Engine::unreachable_for_test(),
        Duration::from_secs(60),
        DEFAULT_WINDOW,
        Upstreams::new(),
        &NOTES.replace(
            "streamable-http}",
            "streamable-http, tools: [search, find]}",
        ),
    )
    .await;
    let tools = s.bus.tools(&s.caller).await;
    let names: Vec<&str> = tools.iter().map(|t| t.name.as_ref()).collect();
    for expected in ["app-example-notes__search", "app-example-notes__find"] {
        assert!(names.contains(&expected), "{names:?}");
    }
    for absent in ["app-example-notes__delete", "app-example-notes__archive"] {
        assert!(!names.contains(&absent), "{names:?}");
    }

    let result = s
        .call("app-example-notes__delete", json!({"name": "x"}))
        .await;
    assert!(failed(&result), "{result:?}");
    assert!(text(&result).contains("没有工具"), "{}", text(&result));
    assert!(s.notes.deleted.lock().unwrap().is_empty(), "请求不能到插件");
}

/// 清单声明的工具上游没有：其余工具照常提供，缺哪些写在 `tools_error` 里，不整个消失。
#[tokio::test]
async fn declared_tools_the_upstream_does_not_have_do_not_hide_the_rest() {
    let s = setup_with(
        Engine::unreachable_for_test(),
        Duration::from_secs(60),
        DEFAULT_WINDOW,
        Upstreams::new(),
        &NOTES.replace(
            "streamable-http}",
            "streamable-http, tools: [search, find, gone]}",
        ),
    )
    .await;
    let tools = s.bus.tools(&s.caller).await;
    let names: Vec<&str> = tools.iter().map(|t| t.name.as_ref()).collect();
    for expected in ["app-example-notes__search", "app-example-notes__find"] {
        assert!(names.contains(&expected), "{names:?}");
    }

    let listed = s.call("wonton__list_plugins", json!({})).await;
    let listed: Value = serde_json::from_str(&text(&listed)).unwrap();
    let plugins = listed["plugins"].as_array().unwrap();
    let plugin = plugins
        .iter()
        .find(|p| p["id"] == "app.example.notes")
        .unwrap();
    let error = plugin["tools_error"].as_str().unwrap();
    assert!(error.contains("清单声明的工具上游没有"), "{error}");
    assert!(error.contains("gone"), "{error}");
}

/// 插件的会话不回答、TCP 没断：只是不列它的工具，别的插件和核心自己的工具照常。
#[tokio::test]
async fn stuck_plugins_do_not_hold_up_everyone_elses_tools() {
    async fn silent_on_lists(body: String) -> axum::response::Response {
        use axum::response::IntoResponse;
        let message: Value = serde_json::from_str(&body).unwrap();
        match message["method"].as_str().unwrap_or_default() {
            "initialize" => axum::Json(json!({
                "jsonrpc": "2.0",
                "id": message["id"],
                "result": {
                    "protocolVersion": "2025-11-25",
                    "capabilities": {"tools": {}},
                    "serverInfo": {"name": "stuck", "version": "1"},
                },
            }))
            .into_response(),
            "tools/list" => std::future::pending().await,
            _ => hyper::StatusCode::ACCEPTED.into_response(),
        }
    }
    let app = axum::Router::new().route("/mcp", axum::routing::post(silent_on_lists));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    const PERIOD: Duration = Duration::from_secs(1);
    let s = setup_with_upstreams(Upstreams::with_tools_timeout(PERIOD)).await;
    for name in ["stuck", "stuck2"] {
        let stuck = NOTES
            .replace("name: notes", &format!("name: {name}"))
            .replace("app.example.notes", &format!("app.example.{name}"));
        let manifest = PluginPackage::parse(&stuck, &Languages::from_header(Some("zh-Hans")))
            .unwrap()
            .manifest;
        let id = manifest.id.clone();
        s.manager
            .register_for_test(manifest, PluginStatus::Running, None);
        s.manager.mcp_for_test(&id, format!("http://{address}/mcp"));
    }

    let started = std::time::Instant::now();
    let tools = s.bus.tools(&s.caller).await;
    assert!(
        started.elapsed() < PERIOD * 2,
        "两个卡住的插件要并发地等，等了 {:?}",
        started.elapsed()
    );
    let names: Vec<_> = tools.iter().map(|t| t.name.to_string()).collect();
    assert!(names.iter().any(|n| n.ends_with("search")), "{names:?}");
    assert!(!names.iter().any(|n| n.contains("stuck")), "{names:?}");

    // 工具不见了要看得到原因，不只是核心日志里有。
    let listed = s.call("wonton__list_plugins", json!({})).await;
    let listed: Value = serde_json::from_str(&text(&listed)).unwrap();
    let error_of = |id: &str| {
        let plugins = listed["plugins"].as_array().unwrap();
        let plugin = plugins.iter().find(|p| p["id"] == id).unwrap();
        plugin["tools_error"].clone()
    };
    assert!(error_of("app.example.notes").is_null());
    assert!(
        error_of("app.example.stuck")
            .as_str()
            .unwrap()
            .contains("没有列出工具"),
        "{listed}"
    );
}

#[tokio::test]
async fn plugin_resources_are_read_with_a_size_limit() {
    let server = McpServer {
        id: PluginId::parse("app.example.notes").unwrap(),
        title: Default::default(),
        url: serve_notes(Notes::default()).await,
        socket: None,
        headers: Default::default(),
        tool_limits: Default::default(),
        tools: None,
    };
    let upstreams = Upstreams::new();
    let read = |uri: &'static str, limit| {
        let (upstreams, server) = (&upstreams, &server);
        async move { upstreams.read_resource(server, uri, limit).await }
    };
    assert_eq!(
        read("notes://text", 64).await.unwrap(),
        "一条笔记".as_bytes()
    );
    assert_eq!(read("notes://blob", 64).await.unwrap(), b"abc");
    assert!(matches!(
        read("notes://blob", 2).await,
        Err(ReadError::TooLarge)
    ));
    assert!(matches!(
        read("notes://missing", 64).await,
        Err(ReadError::Upstream(_))
    ));
    let dead = McpServer {
        url: "http://127.0.0.1:1/mcp".into(),
        ..server.clone()
    };
    let unreachable = upstreams.read_resource(&dead, "notes://text", 64).await;
    assert!(matches!(unreachable, Err(ReadError::Unreachable(_))));
}

mod readiness;

mod uninstall;

mod install;

mod confirmations;

mod manifest;

mod timeouts;
