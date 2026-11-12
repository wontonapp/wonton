//! 总线测试的公共夹具：登记好的假插件、假管理器和卡片，供各测试模块复用。

use std::fs;
use std::os::unix::fs::MetadataExt;
use std::sync::Arc;
use std::time::Duration;

use rmcp::model::{CallToolRequestParams, CallToolResult};
use serde_json::Value;
use wonton_protocol::{Answer, ConfirmationState};

use super::confirm::DEFAULT_WINDOW;
use super::test_support::{Cards, Notes, serve_notes};
use super::upstream::Upstreams;
use super::{AnswerError, Bus, Caller, Confirmations};
use crate::config::PublicUrls;
use crate::http::HttpClient;
use crate::owner::{Owner, OwnerStore};
use crate::plugin::docker::Engine;
use crate::plugin::fake_docker::FakeDocker;
use crate::plugin::image_worker::ImageWorker;
use crate::plugin::languages::Languages;
use crate::plugin::lifecycle::LifecycleConfig;
use crate::plugin::store::Store;
use crate::plugin::storefront::StoreConfig;
use crate::plugin::storefront::index::IndexEntry;
use crate::plugin::{PluginId, PluginManager, PluginPackage, PluginStatus};

/// 提供工具的假插件，登记成在运行。
pub(super) const NOTES: &str = "name: notes
services:
  mcp:
    image: example/notes
x-wonton:
  spec: '0.4'
  id: app.example.notes
  version: 0.1.0
  kind: app
  title: {zh: 笔记, en: Notes}
  description: {zh: 记笔记, en: Notes}
  mcp: {service: mcp, port: 8080, transport: streamable-http}
  permissions: []
";

/// 有最高级权限（docker.socket）的同一个假插件。
pub(super) const ROOT: &str = "name: notes
services:
  mcp:
    image: example/notes
    volumes:
      - /var/run/docker.sock:/var/run/docker.sock
x-wonton:
  spec: '0.5'
  id: app.example.notes
  version: 0.1.0
  kind: app
  title: {zh: 笔记, en: Notes}
  description: {zh: 记笔记, en: Notes}
  mcp: {service: mcp, port: 8080, transport: streamable-http}
  permissions: [docker.socket]
";

/// 官方索引里可以装的插件。
pub(super) fn official_package(id: &str, title: &str, permission: &str) -> String {
    format!(
        "name: app
services:
  web:
    image: nginx
x-wonton:
  spec: '0.4'
  id: {id}
  version: 1.0.0
  kind: app
  title: {{zh: {title}, en: {title}}}
  description: {{zh: {title}的说明, en: About {title}}}
  permissions: [{permission}]
"
    )
}

pub(super) struct Setup {
    pub(super) bus: Arc<Bus>,
    pub(super) cards: Arc<Cards>,
    pub(super) notes: Notes,
    pub(super) manager: Arc<PluginManager>,
    pub(super) caller: Caller,
    pub(super) dir: tempfile::TempDir,
    pub(super) members: Arc<crate::members::Members>,
}

pub(super) fn manager(engine: Engine, dir: &tempfile::TempDir) -> Arc<PluginManager> {
    let owner = fs::metadata(dir.path()).unwrap();
    let cfg = LifecycleConfig {
        public: PublicUrls {
            scheme: "http".into(),
            base_domain: "wonton.localhost".into(),
            port: 8080,
        },
        data_dir: dir.path().to_owned(),
        apps_dir: dir.path().join("apps"),
        shares_dir: dir.path().join("shares"),
        core_container: None,
        tz: "UTC".into(),
        puid: owner.uid(),
        pgid: owner.gid(),
        install_timeout: Duration::from_secs(5),
        owner: "owner".into(),
        profile: OwnerStore::for_test(
            dir.path(),
            &Owner {
                username: "owner".into(),
                display_name: "云吞".into(),
                email: Some("owner@example.com".into()),
            },
        ),
        gateway_port: 8080,
        plugin_slice: None,
        instance: Default::default(),
    };
    let store = Store::open(dir.path().join("plugins")).unwrap();
    let index = StoreConfig {
        index_dir: Some(dir.path().join("index")),
        community: false,
    };
    let manager = PluginManager::new(
        engine,
        store,
        HttpClient::new(),
        None,
        cfg,
        ImageWorker::in_process(),
    );
    Arc::new(manager.with_store(index))
}

/// 官方索引里放一个普通插件（日记）、一个高权限的（共享盘）和一个要核心不开放的权限的。
fn write_index(dir: &tempfile::TempDir) {
    for (repo, source) in [
        (
            "journal",
            official_package(
                "app.example.journal",
                "日记",
                "storage.share.documents:read",
            ),
        ),
        (
            "root",
            official_package("app.example.root", "全能", "docker.socket"),
        ),
        (
            "nas",
            official_package("app.example.nas", "共享盘", "network.lan")
                + "  lan: {ports: [{service: web, port: 445}]}\n",
        ),
    ] {
        let package =
            PluginPackage::parse(&source, &Languages::from_header(Some("zh-Hans"))).unwrap();
        let source = format!("github:example/{repo}");
        let entry = IndexEntry::of(&package, &source, &"a".repeat(40)).unwrap();
        entry.write_to(&dir.path().join("index"), &package).unwrap();
    }
}

pub(super) async fn setup_with_upstreams(upstreams: Upstreams) -> Setup {
    let (_docker, engine) = FakeDocker::start().await;
    setup_with(
        engine,
        Duration::from_secs(5),
        DEFAULT_WINDOW,
        upstreams,
        NOTES,
    )
    .await
}

pub(super) async fn setup(engine: Engine, ttl: Duration) -> Setup {
    setup_with(engine, ttl, DEFAULT_WINDOW, Upstreams::new(), NOTES).await
}

pub(super) async fn setup_window(window: Duration) -> Setup {
    let (_docker, engine) = FakeDocker::start().await;
    setup_with(
        engine,
        Duration::from_secs(5),
        window,
        Upstreams::new(),
        NOTES,
    )
    .await
}

pub(super) async fn setup_root() -> Setup {
    let (_docker, engine) = FakeDocker::start().await;
    setup_with(
        engine,
        Duration::from_secs(5),
        DEFAULT_WINDOW,
        Upstreams::new(),
        ROOT,
    )
    .await
}

/// 有一项可选、默认关的最高级权限（`docker.socket`）的插件。
pub(super) const OPTIONAL: &str = "name: notes
services:
  web:
    image: nginx
    environment:
      STATE: '${WONTON_PERMISSION_DOCKER_SOCKET:-off}'
  agent:
    image: example/agent
    volumes:
      - /var/run/docker.sock:/var/run/docker.sock
x-wonton:
  spec: '0.5'
  id: app.example.notes
  version: 0.1.0
  kind: app
  title: {zh: 笔记, en: Notes}
  description: {zh: 记笔记, en: Notes}
  optional_permissions:
    docker.socket:
      off_loss: {zh: 看不到容器, en: No containers}
  permissions: []
";

/// 装上 [`OPTIONAL`] 的管理器和总线：记录已经写好，可以直接开关。
pub(super) async fn setup_optional() -> Setup {
    let (docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let manager = manager(engine, &dir);
    let package = PluginPackage::parse(OPTIONAL, &Languages::from_header(Some("zh-Hans"))).unwrap();
    manager
        .install_package(package, &[], Languages::from_header(Some("zh-Hans")))
        .await
        .unwrap();
    let cards = Arc::new(Cards::default());
    let confirmations = Arc::new(Confirmations::new(cards.clone(), Duration::from_secs(5)));
    let audit = dir.path().join("audit.jsonl");
    write_index(&dir);
    let bus = Bus::with_upstreams(
        manager.clone(),
        confirmations,
        Arc::new(crate::mcp::audit::Audit::new(audit)),
        Arc::new(Upstreams::new()),
    );
    let members = Arc::new(crate::members::Members::open(dir.path().join("members.json")).unwrap());
    bus.attach_people("wonton.localhost".into(), members.clone());
    let title = [("zh".to_owned(), "测试 Bot".to_owned())].into();
    let caller = Caller::new(PluginId::parse("com.example.agent").unwrap(), Some(&title));
    manager.note_person_languages("owner", Some("zh"));
    let _ = docker;
    Setup {
        bus,
        cards,
        notes: Notes::default(),
        manager,
        caller,
        dir,
        members,
    }
}

pub(super) async fn setup_with(
    engine: Engine,
    ttl: Duration,
    window: Duration,
    upstreams: Upstreams,
    manifest_source: &str,
) -> Setup {
    let dir = tempfile::tempdir().unwrap();
    let manager = manager(engine, &dir);
    let notes = Notes::default();
    let url = serve_notes(notes.clone()).await;
    let manifest = PluginPackage::parse(manifest_source, &Languages::from_header(Some("zh-Hans")))
        .unwrap()
        .manifest;
    let id = manifest.id.clone();
    manager.register_for_test(manifest, PluginStatus::Running, None);
    manager.mcp_for_test(&id, url);
    let cards = Arc::new(Cards::default());
    let confirmations = Arc::new(Confirmations::new(cards.clone(), ttl).with_window_length(window));
    let audit = dir.path().join("audit.jsonl");
    write_index(&dir);
    let bus = Bus::with_upstreams(
        manager.clone(),
        confirmations,
        Arc::new(crate::mcp::audit::Audit::new(audit)),
        Arc::new(upstreams),
    );
    let members = Arc::new(crate::members::Members::open(dir.path().join("members.json")).unwrap());
    bus.attach_people("wonton.localhost".into(), members.clone());
    let title = [("zh".to_owned(), "测试 Bot".to_owned())].into();
    let caller = Caller::new(PluginId::parse("com.example.agent").unwrap(), Some(&title));
    // 主人的语言：生产里由 App 的请求写入，卡片文字按它出中文。
    manager.note_person_languages("owner", Some("zh"));
    Setup {
        bus,
        cards,
        notes,
        manager,
        caller,
        dir,
        members,
    }
}

impl Setup {
    pub(super) async fn call(&self, name: &str, args: Value) -> CallToolResult {
        let request = CallToolRequestParams::new(name.to_owned())
            .with_arguments(args.as_object().cloned().unwrap_or_default());
        self.bus
            .call(&self.caller, request, std::future::pending())
            .await
    }

    /// 以另一个调用方（别的插件）调用。
    pub(super) async fn call_as(&self, caller: &Caller, name: &str, args: Value) -> CallToolResult {
        let request = CallToolRequestParams::new(name.to_owned())
            .with_arguments(args.as_object().cloned().unwrap_or_default());
        self.bus.call(caller, request, std::future::pending()).await
    }

    /// 在后台调用，好在等确认时回答。
    pub(super) fn spawn(&self, name: &str, args: Value) -> tokio::task::JoinHandle<CallToolResult> {
        let (bus, caller, name) = (self.bus.clone(), self.caller.clone(), name.to_owned());
        let request = CallToolRequestParams::new(name)
            .with_arguments(args.as_object().cloned().unwrap_or_default());
        tokio::spawn(async move { bus.call(&caller, request, std::future::pending()).await })
    }

    pub(super) fn answer(
        &self,
        id: &str,
        answer: Answer,
    ) -> Result<ConfirmationState, AnswerError> {
        self.bus.confirmations().answer(id, answer, None)
    }

    pub(super) fn audit(&self) -> Vec<Value> {
        let text = fs::read_to_string(self.dir.path().join("audit.jsonl")).unwrap_or_default();
        text.lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    }
}
