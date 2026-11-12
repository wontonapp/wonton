//! MCP 活动按来源 Bot 计数，确认等待不算执行，后台执行不依赖调用方连接。

use std::fs;
use std::sync::Arc;
use std::time::Duration;

use rmcp::ServerHandler;
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ErrorData, ListToolsResult,
    PaginatedRequestParams, ServerConfig,
};
use rmcp::service::{RequestContext, RoleServer};
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};
use serde_json::json;
use tokio::sync::Semaphore;
use wonton_protocol::{Answer, BotActivityReport, ConfirmationState};

use super::test_setup::{NOTES, Setup, setup};
use super::test_support::Notes;
use super::tests::failed;
use crate::files::test_support::Fixture;
use crate::matrix::BotAccount;
use crate::plugin::bots::BotRecord;
use crate::plugin::docker::Engine;
use crate::plugin::fake_docker::FakeDocker;
use crate::plugin::languages::Languages;
use crate::plugin::store::Store;
use crate::plugin::{PluginId, PluginPackage, PluginStatus};

fn register_bot(s: &Setup) {
    let source = NOTES
        .replace("app.example.notes", s.caller.id.as_str())
        .replace(
            "permissions: []",
            "permissions: [matrix.bot]\n  bot: {display_name: {zh: 来源, en: Source}}",
        );
    s.manager.register_for_test(
        PluginPackage::parse(&source, &Languages::from_header(Some("zh-Hans")))
            .unwrap()
            .manifest,
        PluginStatus::Running,
        None,
    );
    Store::open(s.dir.path().join("plugins"))
        .unwrap()
        .bots()
        .save(
            &s.caller.id,
            &BotRecord {
                account: BotAccount {
                    user_id: "@source:example.com".into(),
                    home_room: "!source:example.com".into(),
                },
                device_id: Some("SOURCE".into()),
            },
        )
        .unwrap();
}

fn report(s: &Setup) -> BotActivityReport {
    s.manager.activity.report(
        s.manager.bot_activity_facts().unwrap(),
        &s.bus.confirmations().pending_by_caller(),
    )
}

fn tool_calls(s: &Setup) -> u32 {
    let report = report(s);
    let source = report
        .bots
        .iter()
        .find(|bot| bot.plugin_id == s.caller.id.as_str())
        .unwrap();
    assert_eq!(source.user_id, "@source:example.com");
    assert!(
        report
            .bots
            .iter()
            .filter(|bot| bot.plugin_id != source.plugin_id)
            .all(|bot| bot.tool_calls == 0)
    );
    source.tool_calls
}

async fn wait_calls(s: &Setup, expected: u32) {
    tokio::time::timeout(Duration::from_secs(5), async {
        while tool_calls(s) != expected {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("工具计数没有到达预期");
}

#[derive(Clone)]
struct Controlled {
    notes: Notes,
    entered: Arc<Semaphore>,
    release: Arc<Semaphore>,
    fail: bool,
}

impl ServerHandler for Controlled {
    fn get_info(&self) -> ServerConfig {
        self.notes.get_info()
    }

    async fn list_tools(
        &self,
        request: Option<PaginatedRequestParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        self.notes.list_tools(request, ctx).await
    }

    async fn call_tool(
        &self,
        _request: CallToolRequestParams,
        _ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        self.entered.add_permits(1);
        self.release.acquire().await.unwrap().forget();
        if self.fail {
            Err(ErrorData::internal_error("工具执行失败", None))
        } else {
            Ok(CallToolResult::success(Vec::new()).into())
        }
    }
}

async fn controlled(s: &Setup, fail: bool) -> Controlled {
    let tool = Controlled {
        notes: Notes::default(),
        entered: Arc::new(Semaphore::new(0)),
        release: Arc::new(Semaphore::new(0)),
        fail,
    };
    let serving = tool.clone();
    let service = StreamableHttpService::new(
        move || Ok(serving.clone()),
        LocalSessionManager::default().into(),
        StreamableHttpServerConfig::default().disable_allowed_hosts(),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let router = axum::Router::new().nest_service("/mcp", service);
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    s.manager.mcp_for_test(
        &PluginId::parse("app.example.notes").unwrap(),
        format!("http://{address}/mcp"),
    );
    tool
}

async fn entered(tool: &Controlled) {
    tokio::time::timeout(Duration::from_secs(5), tool.entered.acquire())
        .await
        .unwrap()
        .unwrap()
        .forget();
}

#[tokio::test]
async fn plugin_execution_success_and_error_release_the_source_bot_activity() {
    for fail in [false, true] {
        let s = setup(Engine::unreachable_for_test(), Duration::from_secs(60)).await;
        register_bot(&s);
        let tool = controlled(&s, fail).await;
        let before = report(&s).rev;
        let call = s.spawn("app-example-notes__search", json!({}));
        entered(&tool).await;
        assert_eq!(tool_calls(&s), 1);
        tool.release.add_permits(1);
        assert_eq!(failed(&call.await.unwrap()), fail);
        assert_eq!(tool_calls(&s), 0);
        assert!(report(&s).rev > before);
    }
}

#[tokio::test]
async fn cancelled_or_dropped_calls_release_activity() {
    let s = setup(Engine::unreachable_for_test(), Duration::from_secs(60)).await;
    register_bot(&s);
    let tool = controlled(&s, false).await;
    let (cancel, cancelled) = tokio::sync::oneshot::channel();
    let (bus, caller) = (s.bus.clone(), s.caller.clone());
    let call = tokio::spawn(async move {
        bus.call(
            &caller,
            CallToolRequestParams::new("app-example-notes__search"),
            async {
                let _ = cancelled.await;
            },
        )
        .await
    });
    entered(&tool).await;
    assert_eq!(tool_calls(&s), 1);
    cancel.send(()).unwrap();
    assert!(failed(&call.await.unwrap()));
    assert_eq!(tool_calls(&s), 0);
    tool.release.add_permits(1);

    let s = setup(Engine::unreachable_for_test(), Duration::from_secs(60)).await;
    register_bot(&s);
    let tool = controlled(&s, false).await;
    let call = s.spawn("app-example-notes__search", json!({}));
    entered(&tool).await;
    assert_eq!(tool_calls(&s), 1);
    call.abort();
    assert!(call.await.unwrap_err().is_cancelled());
    assert_eq!(tool_calls(&s), 0);
    tool.release.add_permits(1);
}

#[tokio::test]
async fn confirmation_wait_is_idle_and_detached_execution_stays_counted() {
    let s = setup(Engine::unreachable_for_test(), Duration::from_secs(60)).await;
    register_bot(&s);
    let tool = controlled(&s, false).await;
    let call = s.spawn("app-example-notes__delete", json!({}));
    let card = s.cards.wait_for(1).await;
    assert_eq!(tool_calls(&s), 0);
    assert_eq!(report(&s).bots[0].waiting, 1);
    s.answer(&card.id, Answer::Approve).unwrap();
    entered(&tool).await;
    assert_eq!(tool_calls(&s), 1);
    assert_eq!(report(&s).bots[0].waiting, 0);
    call.abort();
    assert!(call.await.unwrap_err().is_cancelled());
    assert_eq!(tool_calls(&s), 1);
    tool.release.add_permits(1);
    wait_calls(&s, 0).await;
    tokio::time::timeout(Duration::from_secs(5), async {
        while !matches!(
            s.cards.states(&card.id).last(),
            Some(ConfirmationState::Succeeded { .. })
        ) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn refused_and_expired_confirmations_never_count_as_execution() {
    for (ttl, answer) in [
        (Duration::from_secs(60), Some(Answer::Reject)),
        (Duration::from_millis(100), None),
    ] {
        let s = setup(Engine::unreachable_for_test(), ttl).await;
        register_bot(&s);
        let call = s.spawn("app-example-notes__delete", json!({}));
        let card = s.cards.wait_for(1).await;
        assert_eq!(tool_calls(&s), 0);
        assert_eq!(report(&s).bots[0].waiting, 1);
        if let Some(answer) = answer {
            s.answer(&card.id, answer).unwrap();
        }
        assert!(failed(&call.await.unwrap()));
        assert_eq!(tool_calls(&s), 0);
        assert_eq!(report(&s).bots[0].waiting, 0);
        assert!(s.notes.deleted.lock().unwrap().is_empty());
    }
}

#[tokio::test]
async fn core_install_wait_and_both_highest_cards_are_idle() {
    let (docker, engine) = FakeDocker::start().await;
    let s = setup(engine, Duration::from_secs(60)).await;
    register_bot(&s);
    let package = PluginPackage::parse(
        &super::test_setup::ROOT.replace("app.example.notes", "app.example.root"),
        &Languages::from_header(Some("zh-Hans")),
    )
    .unwrap();
    crate::plugin::storefront::index::IndexEntry::of(
        &package,
        "github:example/root",
        &"a".repeat(40),
    )
    .unwrap()
    .write_to(&s.dir.path().join("index"), &package)
    .unwrap();
    let hold = docker.hold("POST /containers/create");
    let call = s.spawn("wonton__install_plugin", json!({"id": "app.example.root"}));
    let first = s.cards.wait_for(1).await;
    assert_eq!(tool_calls(&s), 0);
    s.answer(&first.id, Answer::Approve).unwrap();
    let second = s.cards.wait_for(2).await;
    assert_eq!(tool_calls(&s), 0);
    s.answer(&second.id, Answer::Approve).unwrap();
    tokio::time::timeout(Duration::from_secs(5), hold.reached())
        .await
        .unwrap();
    assert_eq!(tool_calls(&s), 1);
    call.abort();
    assert!(call.await.unwrap_err().is_cancelled());
    assert_eq!(tool_calls(&s), 1);
    hold.release();
    wait_calls(&s, 0).await;
}

#[tokio::test]
async fn core_save_direct_confirmation_wait_is_idle() {
    let s = setup(Engine::unreachable_for_test(), Duration::from_secs(60)).await;
    register_bot(&s);
    let fixture = Fixture::new();
    s.bus.attach_files(fixture.files.clone());
    fs::create_dir_all(fixture.shares_dir.join("music")).unwrap();
    let outbox = s.manager.app_data_dir(&s.caller.id).join("outbox");
    fs::create_dir_all(&outbox).unwrap();
    fs::write(outbox.join("song.mp3"), "music").unwrap();
    let call = s.spawn(
        "wonton__save_file",
        json!({"files": ["song.mp3"], "share": "music"}),
    );
    let card = s.cards.wait_for(1).await;
    assert_eq!(tool_calls(&s), 0);
    call.abort();
    assert!(call.await.unwrap_err().is_cancelled());
    s.answer(&card.id, Answer::Approve).unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        while !fixture.shares_dir.join("music/song.mp3").exists() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    wait_calls(&s, 0).await;
    assert_eq!(
        fs::read_to_string(fixture.shares_dir.join("music/song.mp3")).unwrap(),
        "music"
    );
}

#[tokio::test]
async fn a_second_highest_card_refusal_never_starts_execution() {
    let (_docker, engine) = FakeDocker::start().await;
    let s = setup(engine, Duration::from_secs(60)).await;
    register_bot(&s);
    let package = PluginPackage::parse(
        &super::test_setup::ROOT.replace("app.example.notes", "app.example.root"),
        &Languages::from_header(Some("zh-Hans")),
    )
    .unwrap();
    crate::plugin::storefront::index::IndexEntry::of(
        &package,
        "github:example/root",
        &"a".repeat(40),
    )
    .unwrap()
    .write_to(&s.dir.path().join("index"), &package)
    .unwrap();
    let call = s.spawn("wonton__install_plugin", json!({"id": "app.example.root"}));
    let first = s.cards.wait_for(1).await;
    assert_eq!(tool_calls(&s), 0);
    s.answer(&first.id, Answer::Approve).unwrap();
    let second = s.cards.wait_for(2).await;
    assert_eq!(tool_calls(&s), 0);
    s.answer(&second.id, Answer::Reject).unwrap();
    assert!(failed(&call.await.unwrap()));
    assert_eq!(tool_calls(&s), 0);
    assert!(
        !s.manager
            .is_installed(&PluginId::parse("app.example.root").unwrap())
    );
}

#[tokio::test]
async fn owner_takeover_wait_is_idle_before_the_confirmation_card() {
    let s = setup(Engine::unreachable_for_test(), Duration::from_secs(60)).await;
    register_bot(&s);
    let tool = controlled(&s, false).await;
    let target = PluginId::parse("app.example.notes").unwrap();
    s.bus.renew_takeover(&target, "device");
    let (holding, held) = tokio::sync::oneshot::channel();
    let (bus, caller) = (s.bus.clone(), s.caller.clone());
    let call = tokio::spawn(async move {
        bus.call(
            &caller,
            CallToolRequestParams::new("app-example-notes__delete"),
            async move {
                holding.send(()).unwrap();
                std::future::pending().await
            },
        )
        .await
    });
    tokio::time::timeout(Duration::from_secs(5), held)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(tool_calls(&s), 0);
    assert!(s.cards.posted.lock().unwrap().is_empty());
    s.bus.end_takeover(&target, "device");
    let card = s.cards.wait_for(1).await;
    assert_eq!(tool_calls(&s), 0);
    s.answer(&card.id, Answer::Approve).unwrap();
    entered(&tool).await;
    assert_eq!(tool_calls(&s), 1);
    tool.release.add_permits(1);
    assert!(!failed(&call.await.unwrap()));
    assert_eq!(tool_calls(&s), 0);
}

#[tokio::test]
async fn ordinary_core_install_tracks_execution_only_after_confirmation() {
    let (docker, engine) = FakeDocker::start().await;
    let s = setup(engine, Duration::from_secs(60)).await;
    register_bot(&s);
    let hold = docker.hold("POST /containers/create");
    let call = s.spawn(
        "wonton__install_plugin",
        json!({"id": "app.example.journal"}),
    );
    let card = s.cards.wait_for(1).await;
    assert_eq!(tool_calls(&s), 0);
    s.answer(&card.id, Answer::Approve).unwrap();
    tokio::time::timeout(Duration::from_secs(5), hold.reached())
        .await
        .unwrap();
    assert_eq!(tool_calls(&s), 1);
    hold.release();
    assert!(!failed(&call.await.unwrap()));
    assert_eq!(tool_calls(&s), 0);
}

#[tokio::test]
async fn core_uninstall_confirmation_wait_is_idle() {
    let s = setup(Engine::unreachable_for_test(), Duration::from_secs(60)).await;
    register_bot(&s);
    let call = s.spawn(
        "wonton__uninstall_plugin",
        json!({"id": "app.example.notes"}),
    );
    let card = s.cards.wait_for(1).await;
    assert_eq!(tool_calls(&s), 0);
    s.answer(&card.id, Answer::Reject).unwrap();
    assert!(failed(&call.await.unwrap()));
    assert_eq!(tool_calls(&s), 0);
}
