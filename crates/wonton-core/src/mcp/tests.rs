//! 总线的行为：工具汇总、只读工具直接执行、会改数据的工具和装卸插件要主人确认、拒绝和过期都不执行。插件的 MCP
//! 服务和卡片通道用 [`test_support`] 里的替身；装卸插件用假 Docker。经网关的 HTTP 入口见 `gateway::mcp_tests`。

use std::fs;
use std::os::unix::fs::MetadataExt;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use rmcp::model::{CallToolRequestParams, CallToolResult};
use serde_json::{Value, json};
use wonton_protocol::{Answer, ConfirmationState};

use super::test_support::{Cards, Notes, serve_notes};
use super::{AnswerError, Bus, Caller, Confirmations};
use crate::config::PublicUrls;
use crate::http::HttpClient;
use crate::plugin::catalog::Catalog;
use crate::plugin::docker::Engine;
use crate::plugin::fake_docker::FakeDocker;
use crate::plugin::image_worker::ImageWorker;
use crate::plugin::lifecycle::LifecycleConfig;
use crate::plugin::store::Store;
use crate::plugin::{PluginId, PluginManager, PluginPackage, PluginStatus};

/// 提供工具的假插件，登记成在运行。
const NOTES: &str = "name: notes
services:
  mcp:
    image: example/notes
x-wonton:
  id: app.example.notes
  version: 0.1.0
  kind: app
  title: {zh: 笔记, en: Notes}
  description: {zh: 记笔记, en: Notes}
  mcp: {service: mcp, port: 8080, transport: streamable-http}
  permissions: []
";

/// 目录里可以装的插件。
fn catalog_package(id: &str, title: &str, permission: &str) -> String {
    format!(
        "name: app
services:
  web:
    image: nginx
x-wonton:
  id: {id}
  version: 1.0.0
  kind: app
  title: {{zh: {title}, en: {title}}}
  description: {{zh: {title}的说明, en: About {title}}}
  permissions: [{permission}]
"
    )
}

struct Setup {
    bus: Arc<Bus>,
    cards: Arc<Cards>,
    notes: Notes,
    manager: Arc<PluginManager>,
    caller: Caller,
    dir: tempfile::TempDir,
}

fn manager(engine: Engine, dir: &tempfile::TempDir) -> Arc<PluginManager> {
    let owner = fs::metadata(dir.path()).unwrap();
    let cfg = LifecycleConfig {
        public: PublicUrls {
            scheme: "http".into(),
            base_domain: "wonton.localhost".into(),
            port: 8080,
        },
        apps_dir: dir.path().join("apps"),
        shares_dir: dir.path().join("shares"),
        core_container: None,
        tz: "UTC".into(),
        puid: owner.uid(),
        pgid: owner.gid(),
        install_timeout: Duration::from_secs(5),
        owner: "owner".into(),
        gateway_port: 8080,
        instance: Default::default(),
    };
    let store = Store::open(dir.path().join("plugins")).unwrap();
    Arc::new(PluginManager::new(
        engine,
        store,
        HttpClient::new(),
        None,
        cfg,
        ImageWorker::in_process(),
    ))
}

/// 目录里放一个普通插件（笔记本）和一个申请高权限的插件。
fn catalog(dir: &tempfile::TempDir) -> Catalog {
    let root = dir.path().join("catalog");
    for (name, source) in [
        (
            "journal",
            catalog_package(
                "app.example.journal",
                "日记",
                "storage.share.documents:read",
            ),
        ),
        (
            "root",
            catalog_package("app.example.root", "全能", "docker.socket"),
        ),
    ] {
        fs::create_dir_all(root.join(name)).unwrap();
        fs::write(root.join(name).join("docker-compose.yml"), source).unwrap();
    }
    Catalog::new(root)
}

async fn setup(engine: Engine, ttl: Duration) -> Setup {
    let dir = tempfile::tempdir().unwrap();
    let manager = manager(engine, &dir);
    let notes = Notes::default();
    let url = serve_notes(notes.clone()).await;
    let manifest = PluginPackage::parse(NOTES).unwrap().manifest;
    let id = manifest.id.clone();
    manager.register_for_test(manifest, PluginStatus::Running, None);
    manager.mcp_for_test(&id, url);
    let cards = Arc::new(Cards::default());
    let confirmations = Arc::new(Confirmations::new(cards.clone(), ttl));
    let audit = dir.path().join("audit.jsonl");
    let bus = Bus::new(manager.clone(), Some(catalog(&dir)), confirmations, audit);
    let title = [("zh".to_owned(), "测试 Bot".to_owned())].into();
    let caller = Caller::new(PluginId::parse("com.example.agent").unwrap(), Some(&title));
    Setup {
        bus,
        cards,
        notes,
        manager,
        caller,
        dir,
    }
}

impl Setup {
    async fn call(&self, name: &str, args: Value) -> CallToolResult {
        let request = CallToolRequestParams::new(name.to_owned())
            .with_arguments(args.as_object().cloned().unwrap_or_default());
        self.bus.call(&self.caller, request).await
    }

    /// 在后台调用，好在等确认时回答。
    fn spawn(&self, name: &str, args: Value) -> tokio::task::JoinHandle<CallToolResult> {
        let (bus, caller, name) = (self.bus.clone(), self.caller.clone(), name.to_owned());
        let request = CallToolRequestParams::new(name)
            .with_arguments(args.as_object().cloned().unwrap_or_default());
        tokio::spawn(async move { bus.call(&caller, request).await })
    }

    fn answer(&self, id: &str, answer: Answer) -> Result<ConfirmationState, AnswerError> {
        self.bus.confirmations().answer(id, answer)
    }

    fn audit(&self) -> Vec<Value> {
        let text = fs::read_to_string(self.dir.path().join("audit.jsonl")).unwrap_or_default();
        text.lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    }
}

fn text(result: &CallToolResult) -> String {
    result
        .content
        .iter()
        .filter_map(|c| c.as_text().map(|t| t.text.clone()))
        .collect()
}

fn failed(result: &CallToolResult) -> bool {
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
        "app-example-notes__search",
        "app-example-notes__delete",
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
    assert_eq!(s.cards.states(&card.id), [ConfirmationState::Rejected]);

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
        failed(&result) && text(&result).contains("没能请主人确认"),
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
        failed(&result) && text(&result).contains("等主人确认"),
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
async fn the_bot_installs_from_the_catalog_after_the_owner_confirms() {
    let (_docker, engine) = FakeDocker::start().await;
    let s = setup(engine, Duration::from_secs(60)).await;
    let found = s
        .call("wonton__search_catalog", json!({"query": "日记"}))
        .await;
    let found: Value = serde_json::from_str(&text(&found)).unwrap();
    assert_eq!(found["plugins"][0]["id"], "app.example.journal");
    assert_eq!(
        found["plugins"][0]["permissions"],
        json!(["读共享文件夹 documents"])
    );

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
    assert!(lines.contains(&"· 读共享文件夹 documents"), "{lines:?}");
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

    // 已经装了、目录里没有、要高权限的：直接拒绝，不发卡片。
    for (id, reason) in [
        ("app.example.journal", "已经装了"),
        ("app.example.nope", "插件目录里没有"),
        ("app.example.root", "只能由主人在 Wonton App 里安装"),
    ] {
        let result = s.call("wonton__install_plugin", json!({"id": id})).await;
        assert!(
            failed(&result) && text(&result).contains(reason),
            "{id}: {result:?}"
        );
    }
    assert_eq!(s.cards.posted.lock().unwrap().len(), 1);
}

#[test]
fn the_install_card_names_the_services_that_get_the_gpu() {
    let compose = catalog_package("app.example.comfy", "画图", "devices.gpu")
        .replace("  permissions:", "  gpu: {services: [web]}\n  permissions:");
    let manifest = PluginPackage::parse(&compose).unwrap().manifest;
    let card = super::cards::install("测试 Bot", &manifest);
    let gpu = card
        .lines
        .iter()
        .find(|l| l.text.contains("显卡交给服务 web"))
        .unwrap_or_else(|| panic!("{:?}", card.lines));
    assert!(gpu.emphasis, "高权限要醒目");
}

#[tokio::test]
async fn uninstalling_needs_the_owner_and_never_removes_the_caller_itself() {
    let (_docker, engine) = FakeDocker::start().await;
    let s = setup(engine, Duration::from_secs(60)).await;
    let package = catalog(&s.dir)
        .find("app.example.journal")
        .unwrap()
        .unwrap();
    let permissions = package.manifest.permissions.clone();
    s.manager
        .install_package(package, &permissions)
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

    let manifest = PluginPackage::parse(&NOTES.replace("app.example.notes", "com.example.agent"))
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
