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

/// 确认过的调用到期了：副作用可能已经发生，卡片和给 Agent 的结果都说不确定，不说失败。Agent 断开时它不跟着取消（ADR-0016）。
#[tokio::test]
async fn a_confirmed_call_that_times_out_leaves_the_card_uncertain() {
    let s = setup(Engine::unreachable_for_test(), Duration::from_secs(60)).await;
    let hanging = Notes::default();
    hanging.hang.store(true, Ordering::SeqCst);
    let manifest = NOTES
        .replace("name: notes", "name: hanging")
        .replace("app.example.notes", "app.example.hanging")
        .replace("streamable-http}", "streamable-http, tool_idle_seconds: 1}");
    let manifest = PluginPackage::parse(&manifest, &Languages::from_header(Some("zh-Hans")))
        .unwrap()
        .manifest;
    let id = manifest.id.clone();
    s.manager
        .register_for_test(manifest, PluginStatus::Running, None);
    s.manager
        .mcp_for_test(&id, serve_notes(hanging.clone()).await);

    let call = s.spawn("app-example-hanging__delete", json!({"name": "a"}));
    let card = s.cards.wait_for(1).await;
    s.answer(&card.id, Answer::Approve).unwrap();
    // 不写外层期限的话，空闲期限被忽略时这里要等 30 分钟的总期限。
    let result = tokio::time::timeout(Duration::from_secs(30), call)
        .await
        .expect("空闲期限到了就要有结果")
        .unwrap();
    assert!(failed(&result), "{result:?}");
    assert!(text(&result).contains("是否已执行不确定"), "{result:?}");
    let meta = result.meta.as_ref().unwrap();
    assert_eq!(meta.0["app.wonton/failure"], "tool_timed_out");
    let states = s.cards.states(&card.id);
    assert!(
        matches!(states.last(), Some(ConfirmationState::Uncertain { .. })),
        "{states:?}"
    );
    assert_eq!(s.audit().last().unwrap()["decision"], "approved");
}

/// 卡片重发到了别的房间：之后的更新发给重发的那张。
#[tokio::test]
async fn a_card_that_moved_is_updated_where_it_now_is() {
    let s = setup(Engine::unreachable_for_test(), Duration::from_secs(60)).await;
    *s.cards.moves_to.lock().unwrap() = Some("$moved".to_owned());
    let call = s.spawn("app-example-notes__delete", json!({"name": "a"}));
    let card = s.cards.wait_for(1).await;
    s.answer(&card.id, Answer::Approve).unwrap();
    call.await.unwrap();
    let handles = s.cards.update_handles.lock().unwrap();
    assert!(handles.len() >= 2, "{handles:?}");
    assert_eq!(handles[0], "$card1");
    assert_eq!(handles[1], "$moved");
}

/// Agent 取消了只读的调用（或者断开了）：取消传给插件，不留一个没人要结果的调用在那里跑。
#[tokio::test]
async fn an_agent_cancelling_a_read_only_call_cancels_it_in_the_plugin() {
    let s = setup(Engine::unreachable_for_test(), Duration::from_secs(60)).await;
    s.notes.hang.store(true, Ordering::SeqCst);
    let request = CallToolRequestParams::new("app-example-notes__search".to_owned());
    let cancelled = tokio::time::sleep(Duration::from_millis(100));
    let result = tokio::time::timeout(
        Duration::from_secs(5),
        s.bus.call(&s.caller, request, cancelled),
    )
    .await
    .expect("取消之后不再等插件");
    assert!(failed(&result), "{result:?}");
    for _ in 0..500 {
        if s.notes.cancelled.load(Ordering::SeqCst) == 1 {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("插件没有收到取消");
}

#[test]
fn a_plugin_can_declare_longer_tool_limits_within_reason() {
    let parse = |extra: &str| {
        let text = NOTES.replace("streamable-http}", &format!("streamable-http{extra}}}"));
        PluginPackage::parse(&text, &Languages::from_header(Some("zh-Hans")))
            .map(|package| package.manifest.mcp.unwrap())
    };
    let mcp = parse(", tool_idle_seconds: 3600, tool_total_seconds: 7200").unwrap();
    assert_eq!(mcp.tool_idle_seconds, Some(3600));
    assert_eq!(mcp.tool_total_seconds, Some(7200));
    assert!(parse(", tool_idle_seconds: 0").is_err());
    assert!(parse(", tool_total_seconds: 99999999").is_err());
    // 总期限先到，空闲期限就白写了。不写总期限时按默认的 30 分钟算。
    assert!(parse(", tool_idle_seconds: 7200, tool_total_seconds: 3600").is_err());
    assert!(parse(", tool_idle_seconds: 7200").is_err());
    assert!(parse(", tool_idle_seconds: 1800").is_ok());
    assert!(parse(", tool_idle_seconds: 3600, tool_total_seconds: 3600").is_ok());
}

/// 清单的 `mcp.tools`：不写时全部提供；写了就必须是非空、不重复的合法工具名列表（MCP 2025-11-25 的
/// 工具名规定，见 `manifest_mcp.rs`）。
#[test]
fn a_plugin_can_limit_its_tools_to_a_valid_allowlist() {
    let parse = |extra: &str| {
        let text = NOTES.replace("streamable-http}", &format!("streamable-http{extra}}}"));
        PluginPackage::parse(&text, &Languages::from_header(Some("zh-Hans")))
            .map(|package| package.manifest.mcp.unwrap())
    };
    let mcp = parse(", tools: [search, find, a.b_c-d]").unwrap();
    assert_eq!(
        mcp.tools,
        Some(vec!["search".into(), "find".into(), "a.b_c-d".into()])
    );
    // 不写这个字段就是全部提供，和现在一样。
    assert_eq!(parse("").unwrap().tools, None);
    for (extra, why) in [
        (", tools: []", "mcp.tools"),
        (", tools: [search, search]", "duplicated"),
        (", tools: [\"search files\"]", "not a valid tool name"),
        (", tools: [搜索]", "not a valid tool name"),
    ] {
        let error = parse(extra).unwrap_err().to_string();
        assert!(error.contains(why), "{extra}: {error}");
    }
}

#[tokio::test]
async fn read_only_tools_run_without_asking_the_owner() {
    let s = setup(Engine::unreachable_for_test(), Duration::from_secs(60)).await;
    let result = s
        .call("app-example-notes__search", json!({"name": "a"}))
        .await;
    assert!(!failed(&result), "{result:?}");
    assert_eq!(text(&result), "找到 a.txt");
    assert!(
        s.cards.posted.lock().unwrap().is_empty(),
        "只读工具不发卡片"
    );
    let unknown = s.call("app-example-notes__nope", json!({})).await;
    assert!(failed(&unknown));
    let decisions: Vec<Value> = s.audit().iter().map(|l| l["decision"].clone()).collect();
    assert_eq!(decisions, [json!("allowed"), json!("denied")]);
}

/// 同为同意的按钮靠 `choice_id` 区分：回答要对得上卡片上的某一个（ADR-0035）。
#[tokio::test]
async fn an_answer_has_to_match_a_button_on_the_card() {
    let s = setup(Engine::unreachable_for_test(), Duration::from_secs(60)).await;
    let button = |answer, id: &str| CardChoice {
        answer,
        label: id.to_owned(),
        choice_id: Some(id.to_owned()),
    };
    let draft = Draft {
        title: "Agent 想运行 ls".to_owned(),
        lines: Vec::new(),
        choices: vec![
            button(Answer::Approve, "once"),
            button(Answer::Approve, "always"),
            button(Answer::Reject, "no"),
        ],
        scope: None,
        plugin_id: None,
        secret_inputs: Vec::new(),
        settings: None,
    };
    let (bus, caller) = (s.bus.clone(), s.caller.id.clone());
    let asked = tokio::spawn(async move { bus.confirmations().ask(&caller, draft).await.is_ok() });
    let card = s.cards.wait_for(1).await;
    let answer = |answer, choice| s.bus.confirmations().answer(&card.id, answer, choice);

    for (wrong_answer, wrong_choice) in [
        (Answer::Approve, None),
        (Answer::Approve, Some("nope")),
        (Answer::Approve, Some("no")),
        (Answer::Reject, Some("once")),
    ] {
        assert_eq!(
            answer(wrong_answer, wrong_choice),
            Err(AnswerError::UnknownAnswer),
            "{wrong_answer:?} {wrong_choice:?}"
        );
    }
    assert_eq!(
        answer(Answer::Approve, Some("always")),
        Ok(ConfirmationState::Running)
    );
    assert!(asked.await.unwrap());
}

#[tokio::test]
async fn writes_run_only_after_the_owner_approves() {
    let s = setup(Engine::unreachable_for_test(), Duration::from_secs(60)).await;
    let call = s.spawn("app-example-notes__delete", json!({"name": "b"}));
    let card = s.cards.wait_for(1).await;
    assert_eq!(card.state, ConfirmationState::Pending);
    assert!(
        card.title.contains("测试 Bot") && card.title.contains("笔记"),
        "{}",
        card.title
    );
    assert!(
        s.notes.deleted.lock().unwrap().is_empty(),
        "主人回答之前不执行"
    );
    assert_eq!(
        s.answer(&card.id, Answer::ApproveWider),
        Err(AnswerError::UnknownAnswer),
        "卡片上没有的按钮不算回答"
    );
    assert_eq!(
        s.answer(&card.id, Answer::Approve),
        Ok(ConfirmationState::Running)
    );
    let result = call.await.unwrap();
    assert_eq!(text(&result), "删掉了 b.txt");
    assert_eq!(*s.notes.deleted.lock().unwrap(), ["b"]);
    let states = s.cards.states(&card.id);
    assert_eq!(states.first(), Some(&ConfirmationState::Running));
    assert!(
        matches!(states.last(), Some(ConfirmationState::Succeeded { .. })),
        "{states:?}"
    );
    assert_eq!(
        s.answer(&card.id, Answer::Reject),
        Err(AnswerError::AlreadyAnswered)
    );
    assert_eq!(
        s.answer("nope", Answer::Approve),
        Err(AnswerError::NotFound)
    );
}

#[tokio::test]
async fn rejected_and_expired_requests_never_run() {
    let s = setup(Engine::unreachable_for_test(), Duration::from_millis(400)).await;
    let call = s.spawn("app-example-notes__delete", json!({"name": "c"}));
    let card = s.cards.wait_for(1).await;
    s.answer(&card.id, Answer::Reject).unwrap();
    let result = call.await.unwrap();
    assert!(
        failed(&result) && text(&result).contains("拒绝"),
        "{result:?}"
    );
    let states = s.cards.states(&card.id);
    assert!(!states.is_empty());
    assert!(
        states
            .iter()
            .all(|state| *state == ConfirmationState::Rejected)
    );

    let result = s
        .call("app-example-notes__delete", json!({"name": "d"}))
        .await;
    assert!(
        failed(&result) && text(&result).contains("过期"),
        "{result:?}"
    );
    let expired = s.cards.wait_for(2).await;
    assert_eq!(s.cards.states(&expired.id), [ConfirmationState::Expired]);
    assert_eq!(
        s.answer(&expired.id, Answer::Approve),
        Err(AnswerError::Expired)
    );
    assert!(s.notes.deleted.lock().unwrap().is_empty());
    let decisions: Vec<Value> = s.audit().iter().map(|l| l["decision"].clone()).collect();
    assert_eq!(decisions, [json!("rejected"), json!("expired")]);
}

#[tokio::test]
async fn without_a_card_nothing_runs() {
    let s = setup(Engine::unreachable_for_test(), Duration::from_secs(60)).await;
    s.cards.broken.store(true, Ordering::SeqCst);
    let result = s
        .call("app-example-notes__delete", json!({"name": "e"}))
        .await;
    assert!(
        failed(&result) && text(&result).contains("没能请你确认"),
        "{result:?}"
    );
    s.cards.broken.store(false, Ordering::SeqCst);
    // 同一个插件最多三个请求同时等主人。
    let waiting: Vec<_> = (0..3)
        .map(|i| s.spawn("app-example-notes__delete", json!({"name": i})))
        .collect();
    s.cards.wait_for(3).await;
    let result = s
        .call("app-example-notes__delete", json!({"name": "f"}))
        .await;
    assert!(
        failed(&result) && text(&result).contains("等你确认"),
        "{result:?}"
    );
    for card in s.cards.posted.lock().unwrap().iter() {
        s.answer(&card.id, Answer::Reject).unwrap();
    }
    for call in waiting {
        assert!(failed(&call.await.unwrap()));
    }
    assert!(s.notes.deleted.lock().unwrap().is_empty());
}

#[tokio::test]
async fn the_bot_installs_from_the_official_index_after_the_owner_confirms() {
    let (_docker, engine) = FakeDocker::start().await;
    let s = setup(engine, Duration::from_secs(60)).await;
    let found = s
        .call("wonton__search_catalog", json!({"query": "日记"}))
        .await;
    let found: Value = serde_json::from_str(&text(&found)).unwrap();
    assert_eq!(found["plugins"][0]["id"], "app.example.journal");
    // 主人没存过语言，回退英文；期望值用目录本身格式化出来。
    let langs = s.bus.plugins.owner_languages();
    let share = crate::l10n::text(
        &langs,
        "permission-storage-share",
        &[("access", "read".into()), ("name", "documents".into())],
    );
    assert_eq!(found["plugins"][0]["permissions"], json!([share]));

    let call = s.spawn(
        "wonton__install_plugin",
        json!({"id": "app.example.journal"}),
    );
    let card = s.cards.wait_for(1).await;
    assert!(
        card.title.contains("测试 Bot") && card.title.contains("日记"),
        "{}",
        card.title
    );
    let lines: Vec<&str> = card.lines.iter().map(|l| l.text.as_str()).collect();
    assert!(lines.contains(&format!("· {share}").as_str()), "{lines:?}");
    assert!(
        s.manager
            .list()
            .iter()
            .all(|p| p.id.as_str() != "app.example.journal")
    );
    s.answer(&card.id, Answer::Approve).unwrap();
    let result = call.await.unwrap();
    assert!(!failed(&result), "{result:?}");
    let installed = s.manager.list();
    let journal = installed
        .iter()
        .find(|p| p.id.as_str() == "app.example.journal");
    assert_eq!(
        journal.map(|p| p.status.clone()),
        Some(PluginStatus::Running)
    );
    let states = s.cards.states(&card.id);
    assert!(
        matches!(states.last(), Some(ConfirmationState::Succeeded { .. })),
        "{states:?}"
    );

    // 已经装了、官方索引里没有、要核心不开放的权限的：直接拒绝，不发卡片。
    for (id, reason) in [
        ("app.example.journal", "已经装了"),
        ("app.example.nope", "官方索引里没有"),
        ("app.example.root", "docker.sock 挂载"),
    ] {
        let result = s.call("wonton__install_plugin", json!({"id": id})).await;
        assert!(
            failed(&result) && text(&result).contains(reason),
            "{id}: {result:?}"
        );
    }
    assert_eq!(s.cards.posted.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn a_bot_installs_a_high_permission_plugin_once_the_owner_approves_the_card() {
    let (_docker, engine) = FakeDocker::start().await;
    let s = setup(engine, Duration::from_secs(60)).await;

    let call = s.spawn("wonton__install_plugin", json!({"id": "app.example.nas"}));
    let card = s.cards.wait_for(1).await;
    assert!(
        card.lines.iter().any(|l| l.emphasis),
        "高权限要醒目：{:?}",
        card.lines
    );
    s.answer(&card.id, Answer::Approve).unwrap();
    let result = call.await.unwrap();
    assert!(!failed(&result), "{result:?}");
    assert!(
        s.manager
            .list()
            .iter()
            .any(|p| p.id.as_str() == "app.example.nas")
    );
}

#[tokio::test]
async fn an_unreadable_index_is_reported_not_shown_as_empty() {
    let (_docker, engine) = FakeDocker::start().await;
    let s = setup(engine, Duration::from_secs(60)).await;
    fs::write(s.dir.path().join("index").join("index.json"), "{").unwrap();
    for (tool, args) in [
        ("wonton__search_catalog", json!({})),
        (
            "wonton__install_plugin",
            json!({"id": "app.example.journal"}),
        ),
    ] {
        let result = s.call(tool, args).await;
        assert!(
            failed(&result) && text(&result).contains("官方索引用不了"),
            "{tool}: {result:?}"
        );
    }
    assert!(s.cards.posted.lock().unwrap().is_empty());
}

#[tokio::test]
async fn uninstalling_needs_the_owner_and_never_removes_the_caller_itself() {
    let (_docker, engine) = FakeDocker::start().await;
    let s = setup(engine, Duration::from_secs(60)).await;
    let package = PluginPackage::parse(
        &official_package(
            "app.example.journal",
            "日记",
            "storage.share.documents:read",
        ),
        &Languages::from_header(Some("zh-Hans")),
    )
    .unwrap();
    let permissions = package.manifest.permissions.clone();
    s.manager
        .install_package(
            package,
            &permissions,
            Languages::from_header(Some("zh-Hans")),
        )
        .await
        .unwrap();

    let call = s.spawn(
        "wonton__uninstall_plugin",
        json!({"id": "app.example.journal", "delete_data": true}),
    );
    let card = s.cards.wait_for(1).await;
    assert!(
        card.lines
            .iter()
            .any(|l| l.emphasis && l.text.contains("删除它的数据"))
    );
    s.answer(&card.id, Answer::Approve).unwrap();
    let result = call.await.unwrap();
    assert!(!failed(&result), "{result:?}");
    assert!(
        s.manager
            .list()
            .iter()
            .all(|p| p.id.as_str() != "app.example.journal")
    );

    let manifest = PluginPackage::parse(
        &NOTES.replace("app.example.notes", "com.example.agent"),
        &Languages::from_header(Some("zh-Hans")),
    )
    .unwrap()
    .manifest;
    s.manager
        .register_for_test(manifest, PluginStatus::Running, None);
    let result = s
        .call(
            "wonton__uninstall_plugin",
            json!({"id": "com.example.agent"}),
        )
        .await;
    assert!(
        failed(&result) && text(&result).contains("不能卸载自己"),
        "{result:?}"
    );
    assert_eq!(s.cards.posted.lock().unwrap().len(), 1);
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
