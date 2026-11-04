//! 假扮的核心管理接口和驱动 App 的工具，只走回环地址。

use std::collections::{HashMap, VecDeque};
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::extract::{Request, State};
use axum::http::{Method, StatusCode, header};
use axum::response::{IntoResponse, Response};
use iroh::endpoint::presets;
use iroh::{Endpoint, EndpointId, SecretKey};
use tokio::net::TcpListener;
use wonton_client::{
    Action, ClientCore, HomeStatus, InstallPreview, InstalledPlugin, InstalledPlugins, Network,
    PermissionLine, PluginHealth, PluginKind, PluginState, PluginTaskState, PluginTrust,
    PluginsViewModel, StoreItem, StoreListing, StoreSection,
};
use wonton_tunnel::{Allowlist, TunnelServer};

pub const WAIT: Duration = Duration::from_secs(20);
pub const IMAGE: &str = "/_wonton/api/images/abc/icon-192.png";

/// 核心对一个请求的预设回应，排在队列里，用完后回默认的。
#[derive(Clone)]
pub struct Canned(pub u16, pub String);

pub fn core_error(status: u16, kind: &str, message: &str) -> Canned {
    let body = serde_json::json!({"error": {"kind": kind, "message": message}});
    Canned(status, body.to_string())
}

#[derive(Default)]
pub struct CoreState {
    /// 依次给出的已安装列表，最后一份一直用下去。
    lists: Mutex<VecDeque<Vec<InstalledPlugin>>>,
    /// 某个方法和路径下一次的回应：App 还会向核心要 Matrix 登录令牌，预设的回应不能让别的请求用掉。
    canned: Mutex<HashMap<(Method, String), Canned>>,
    seen: Mutex<Vec<Seen>>,
}

#[derive(Clone, Debug)]
pub struct Seen {
    pub method: Method,
    pub path: String,
    pub language: Option<String>,
    pub from_page: bool,
    pub body: String,
}

impl CoreState {
    pub fn answer(&self, method: Method, path: &str, canned: Canned) {
        self.canned
            .lock()
            .unwrap()
            .insert((method, path.to_owned()), canned);
    }

    pub fn set_lists(&self, lists: Vec<Vec<InstalledPlugin>>) {
        *self.lists.lock().unwrap() = lists.into();
    }

    pub fn seen(&self, method: &Method, path: &str) -> Vec<Seen> {
        self.seen
            .lock()
            .unwrap()
            .iter()
            .filter(|seen| seen.method == *method && seen.path == path)
            .cloned()
            .collect()
    }
}

pub async fn fake_core(State(core): State<Arc<CoreState>>, request: Request) -> Response {
    let (parts, body) = request.into_parts();
    let path = parts
        .uri
        .path_and_query()
        .map_or(String::new(), ToString::to_string);
    let body = to_bytes(body, usize::MAX).await.unwrap();
    core.seen.lock().unwrap().push(Seen {
        method: parts.method.clone(),
        path: path.clone(),
        language: parts
            .headers
            .get(header::ACCEPT_LANGUAGE)
            .map(|v| v.to_str().unwrap().to_owned()),
        from_page: parts.headers.contains_key(header::ORIGIN)
            || parts.headers.contains_key("sec-fetch-site"),
        body: String::from_utf8_lossy(&body).into_owned(),
    });
    if path == IMAGE {
        return (StatusCode::OK, vec![0x89, 0x50]).into_response();
    }
    let canned = core
        .canned
        .lock()
        .unwrap()
        .remove(&(parts.method.clone(), parts.uri.path().to_owned()));
    if let Some(Canned(status, body)) = canned {
        return Response::builder()
            .status(status)
            .body(Body::from(body))
            .unwrap();
    }
    let json = |value: String| (StatusCode::OK, value).into_response();
    match (&parts.method, parts.uri.path()) {
        (&Method::GET, "/_wonton/api/plugins") => {
            let mut lists = core.lists.lock().unwrap();
            let plugins = if lists.len() > 1 {
                lists.pop_front().unwrap()
            } else {
                lists.front().cloned().unwrap_or_default()
            };
            json(
                serde_json::to_string(&InstalledPlugins {
                    plugins,
                    upgrade_check: None,
                })
                .unwrap(),
            )
        }
        (&Method::GET, "/_wonton/api/store") => json(serde_json::to_string(&listing()).unwrap()),
        (&Method::POST, "/_wonton/api/plugins/preview") => {
            json(serde_json::to_string(&preview("app.photos")).unwrap())
        }
        (&Method::POST, "/_wonton/api/plugins") => json(
            serde_json::to_string(&installed(
                "app.photos",
                PluginState::Running {
                    health: PluginHealth::Healthy,
                },
            ))
            .unwrap(),
        ),
        (&Method::DELETE, _) => json("{}".to_owned()),
        _ => {
            let Canned(status, body) = core_error(404, "not_found", "没有这个");
            (StatusCode::from_u16(status).unwrap(), body).into_response()
        }
    }
}

pub fn installed(id: &str, status: PluginState) -> InstalledPlugin {
    InstalledPlugin {
        id: id.to_owned(),
        title: format!("标题 {id}"),
        summary: String::new(),
        version: "1.0.0".to_owned(),
        kind: PluginKind::App,
        icon: None,
        trust: PluginTrust::Official,
        source: None,
        provider: None,
        via_index: None,
        status,
        web_url: None,
        web_display: wonton_protocol::TabDisplay::default(),
        tabs: Vec::new(),
        bot_user_id: None,
        upgrade: None,
        removable: true,
        notices: Vec::new(),
    }
}

pub fn running(id: &str) -> InstalledPlugin {
    installed(
        id,
        PluginState::Running {
            health: PluginHealth::Healthy,
        },
    )
}

pub fn preview(id: &str) -> InstallPreview {
    InstallPreview {
        source: Some(format!("github:owner/{id}@v1.0.0")),
        commit: None,
        digest: "sha256:abc".to_owned(),
        id: id.to_owned(),
        title: format!("标题 {id}"),
        summary: String::new(),
        version: "1.0.0".to_owned(),
        kind: PluginKind::App,
        icon: None,
        trust: PluginTrust::Official,
        provider: None,
        via_index: None,
        author: None,
        permissions: vec![PermissionLine {
            permission: "storage.share.photos:read".to_owned(),
            text: "读共享文件夹 photos".to_owned(),
            high: false,
            supported: true,
            added: false,
        }],
        notices: Vec::new(),
        upgrade_from: None,
        release_notes: None,
        blocked: None,
        missing_profile: None,
        lan_webui: None,
    }
}

pub fn item(id: &str, title: &str, tags: &[&str]) -> StoreItem {
    StoreItem {
        source: format!("github:owner/{id}"),
        id: id.to_owned(),
        title: title.to_owned(),
        summary: String::new(),
        icon: None,
        tags: tags.iter().map(ToString::to_string).collect(),
        kind: PluginKind::App,
        version: "1.0.0".to_owned(),
        trust: PluginTrust::Official,
        provider: None,
        via_index: None,
        author: None,
        stars: None,
        installed_version: None,
        high_permissions: false,
        blocked: None,
    }
}

pub fn listing() -> StoreListing {
    let section = |plugins| StoreSection {
        plugins,
        fetched_at_ms: None,
        problem: None,
    };
    StoreListing {
        official: section(vec![item("app.photos", "Photos", &["相册"])]),
        added: Vec::new(),
        community: section(vec![
            item("app.music", "Music", &["音乐"]),
            item("app.more-photos", "Gallery", &["PHOTOS"]),
        ]),
    }
}

/// 假扮的家里服务器：隧道的对面是假核心。
pub struct FakeHome {
    pub core: Arc<CoreState>,
    pub allowed: Allowlist,
    server: TunnelServer,
}

impl FakeHome {
    pub async fn start(allowed: Vec<EndpointId>) -> Self {
        let core = Arc::new(CoreState::default());
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let upstream = listener.local_addr().unwrap();
        let router = Router::new().fallback(fake_core).with_state(core.clone());
        tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        let endpoint = Endpoint::builder(presets::Minimal)
            .secret_key(SecretKey::generate())
            .clear_ip_transports()
            .bind_addr((Ipv4Addr::LOCALHOST, 0))
            .unwrap()
            .bind()
            .await
            .unwrap();
        let allowed = Allowlist::new(allowed);
        let server = TunnelServer::spawn(endpoint, allowed.clone(), upstream);
        Self {
            core,
            allowed,
            server,
        }
    }

    pub fn ticket(&self) -> String {
        ticket(
            self.server.endpoint().id(),
            self.server.endpoint().bound_sockets()[0],
        )
    }
}

pub fn ticket(id: EndpointId, addr: SocketAddr) -> String {
    format!(
        r#"{{"server": {{"endpoint_id": "{id}", "direct_addrs": ["{addr}"]}},
            "base_domain": "wonton.localhost"}}"#
    )
}

pub struct Rig {
    pub core: Arc<CoreState>,
    pub app: Arc<ClientCore>,
    _home: FakeHome,
    pub home_ticket: String,
    _dir: tempfile::TempDir,
}

impl Rig {
    pub async fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let app =
            ClientCore::with_network(dir.path().to_str().unwrap(), Network::DirectOnly).unwrap();
        let device = app.home().device_id.parse().unwrap();
        let home = FakeHome::start(vec![device]).await;
        Self {
            core: home.core.clone(),
            app,
            home_ticket: home.ticket(),
            _home: home,
            _dir: dir,
        }
    }

    /// 关掉 App 再打开。
    pub fn reopen(&mut self) {
        self.app =
            ClientCore::with_network(self._dir.path().to_str().unwrap(), Network::DirectOnly)
                .unwrap();
    }

    pub fn device(&self) -> EndpointId {
        self.app.home().device_id.parse().unwrap()
    }

    pub fn connect(&self) {
        self.connect_to(self.home_ticket.clone());
    }

    pub fn connect_to(&self, ticket: String) {
        self.app.dispatch(Action::ConnectHome { ticket }).unwrap();
    }

    pub async fn connected(&self) {
        self.connect();
        let deadline = Instant::now() + WAIT;
        while self.app.home().status != HomeStatus::Connected || self.app.home().proxy.is_none() {
            assert!(Instant::now() < deadline, "{:?}", self.app.home());
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    pub async fn wait_for(
        &self,
        mut done: impl FnMut(&PluginsViewModel) -> bool,
    ) -> PluginsViewModel {
        let deadline = Instant::now() + WAIT;
        loop {
            let view = self.app.plugins();
            if done(&view) {
                return view;
            }
            assert!(Instant::now() < deadline, "{view:?}");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    pub async fn task(
        &self,
        id: &str,
        mut done: impl FnMut(&PluginTaskState) -> bool,
    ) -> PluginTaskState {
        let view = self
            .wait_for(|view| {
                view.tasks
                    .iter()
                    .any(|task| task.id == id && done(&task.state))
            })
            .await;
        view.tasks
            .into_iter()
            .find(|task| task.id == id)
            .unwrap()
            .state
    }
}
