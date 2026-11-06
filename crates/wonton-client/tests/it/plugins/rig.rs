//! 假扮的核心管理接口和驱动 App 的工具，只走回环地址。

use std::collections::{HashMap, VecDeque};
use std::net::{Ipv4Addr, SocketAddr};
use std::path::Path;
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
use tokio::sync::watch;
use wonton_client::{
    Action, ClientCore, HomeStatus, InstallPreview, InstalledPlugin, InstalledPlugins, Network,
    OnboardingViewModel, PermissionLine, PluginHealth, PluginKind, PluginState, PluginTaskState,
    PluginTrust, PluginsViewModel, StoreItem, StoreListing, StoreSection,
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

fn json_value(value: serde_json::Value) -> Response {
    (StatusCode::OK, value.to_string()).into_response()
}

fn core_error_response(status: u16) -> Response {
    let Canned(status, body) = core_error(status, "not_found", "没有这个");
    (StatusCode::from_u16(status).unwrap(), body).into_response()
}

/// 按住不回应的路径：测试要卡住一次请求时（如保存 key 途中再存一次），放行后才回。
type Holds = Mutex<HashMap<(Method, String), Arc<watch::Sender<bool>>>>;

#[derive(Default)]
pub struct CoreState {
    /// 依次给出的已安装列表，最后一份一直用下去。
    lists: Mutex<VecDeque<Vec<InstalledPlugin>>>,
    /// 假核心保存的引导状态和服务商列表。
    onboarding: Mutex<Option<serde_json::Value>>,
    providers: Mutex<Option<serde_json::Value>>,
    /// 假核心保存的资料；为空时是还没填的。
    owner: Mutex<Option<serde_json::Value>>,
    /// 某个方法和路径下一次的回应：App 还会向核心要 Matrix 登录令牌，预设的回应不能让别的请求用掉。
    canned: Mutex<HashMap<(Method, String), Canned>>,
    /// 用完预设的回应后，某个方法和路径一直用的回应。
    standing: Mutex<HashMap<(Method, String), Canned>>,
    /// 按住不回应的路径：测试要卡住一次请求时（如保存 key 途中再存一次），放行后才回。
    holds: Holds,
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

    pub fn answer_always(&self, method: Method, path: &str, canned: Canned) {
        self.standing
            .lock()
            .unwrap()
            .insert((method, path.to_owned()), canned);
    }

    pub fn stop_answering(&self, method: Method, path: &str) {
        self.standing
            .lock()
            .unwrap()
            .remove(&(method, path.to_owned()));
    }

    /// 让这个请求一直等，直到 [`Self::release`]。
    pub fn hold(&self, method: Method, path: &str) {
        self.holds
            .lock()
            .unwrap()
            .insert((method, path.to_owned()), Arc::new(watch::channel(true).0));
    }

    pub fn release(&self, method: Method, path: &str) {
        if let Some(held) = self.holds.lock().unwrap().get(&(method, path.to_owned())) {
            held.send_replace(false);
        }
    }

    pub fn set_lists(&self, lists: Vec<Vec<InstalledPlugin>>) {
        *self.lists.lock().unwrap() = lists.into();
    }

    pub fn set_onboarding(&self, onboarding: serde_json::Value) {
        *self.onboarding.lock().unwrap() = Some(onboarding);
    }

    pub fn set_owner(&self, name: Option<&str>, email: Option<&str>) {
        *self.owner.lock().unwrap() = Some(owner_json(name, email));
    }

    pub fn set_providers(&self, providers: serde_json::Value) {
        *self.providers.lock().unwrap() = Some(providers);
    }

    pub fn all_seen(&self) -> Vec<Seen> {
        self.seen.lock().unwrap().clone()
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
    let held = core
        .holds
        .lock()
        .unwrap()
        .get(&(parts.method.clone(), parts.uri.path().to_owned()))
        .cloned();
    if let Some(held) = held {
        let mut released = held.subscribe();
        released.wait_for(|held| !*held).await.ok();
    }
    if path == IMAGE {
        return (StatusCode::OK, vec![0x89, 0x50]).into_response();
    }
    let canned = core
        .canned
        .lock()
        .unwrap()
        .remove(&(parts.method.clone(), parts.uri.path().to_owned()))
        .or_else(|| {
            core.standing
                .lock()
                .unwrap()
                .get(&(parts.method.clone(), parts.uri.path().to_owned()))
                .cloned()
        });
    if let Some(Canned(status, body)) = canned {
        return Response::builder()
            .status(status)
            .body(Body::from(body))
            .unwrap();
    }
    if let Some(response) = onboarding_routes(&core, &parts.method, parts.uri.path(), &body).await {
        return response;
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
            // 来源 github:owner/<id> 预览出来就是 <id>。
            let id = serde_json::from_slice::<serde_json::Value>(&body)
                .ok()
                .and_then(|body| body["source"].as_str().map(str::to_owned))
                .and_then(|source| source.strip_prefix("github:owner/").map(str::to_owned))
                .unwrap_or_else(|| "app.photos".to_owned());
            json(serde_json::to_string(&preview(&id)).unwrap())
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

async fn onboarding_routes(
    core: &Arc<CoreState>,
    method: &Method,
    path: &str,
    body: &[u8],
) -> Option<Response> {
    if path == "/_wonton/api/onboarding" {
        return Some(match *method {
            Method::GET => core
                .onboarding
                .lock()
                .unwrap()
                .clone()
                .map(json_value)
                .unwrap_or_else(|| core_error_response(404)),
            Method::PUT => {
                let mut state = core.onboarding.lock().unwrap();
                let Some(value) = state.as_mut() else {
                    return Some(core_error_response(404));
                };
                if let Ok(update) = serde_json::from_slice::<serde_json::Value>(body) {
                    for field in ["step", "primary_agent", "replaced_agent_asked"] {
                        if let Some(next) = update.get(field).filter(|value| !value.is_null()) {
                            value[field] = next.clone();
                        }
                    }
                }
                json_value(value.clone())
            }
            _ => core_error_response(405),
        });
    }
    if path == "/_wonton/api/models/providers" {
        return Some(match *method {
            Method::GET => core
                .providers
                .lock()
                .unwrap()
                .clone()
                .map(json_value)
                .unwrap_or_else(|| core_error_response(404)),
            _ => core_error_response(405),
        });
    }
    if path == "/_wonton/api/owner-profile" {
        let mut owner = core.owner.lock().unwrap();
        let value = owner.get_or_insert_with(|| owner_json(None, None));
        if *method == Method::PUT
            && let Ok(update) = serde_json::from_slice::<serde_json::Value>(body)
        {
            value["display_name"] = update["display_name"].clone();
            value["email"] = update.get("email").cloned().unwrap_or_default();
        }
        return Some(json_value(value.clone()));
    }
    // 只测不存。
    if path.starts_with("/_wonton/api/models/") && path.ends_with("/test") {
        return Some(StatusCode::OK.into_response());
    }
    if path == "/_wonton/api/models/custom-provider" && method == Method::PUT {
        return Some(StatusCode::OK.into_response());
    }
    if path.starts_with("/_wonton/api/models/providers/") && method == Method::PUT {
        if let Some(providers) = core.providers.lock().unwrap().as_mut() {
            providers["any_configured"] = serde_json::Value::Bool(true);
            if let Some(items) = providers
                .get_mut("providers")
                .and_then(|items| items.as_array_mut())
            {
                for provider in items {
                    provider["configured"] = serde_json::Value::Bool(true);
                }
            }
        }
        return Some(StatusCode::OK.into_response());
    }
    None
}

fn owner_json(name: Option<&str>, email: Option<&str>) -> serde_json::Value {
    serde_json::json!({"username": "owner", "display_name": name, "email": email})
}

pub fn installed(id: &str, status: PluginState) -> InstalledPlugin {
    InstalledPlugin {
        id: id.to_owned(),
        title: format!("标题 {id}"),
        summary: String::new(),
        version: "1.0.0".to_owned(),
        revision: "1.0.0".to_owned(),
        kind: PluginKind::App,
        icon: None,
        trust: PluginTrust::Official,
        source: None,
        provider: None,
        via_index: None,
        written_by: None,
        status,
        web_url: None,
        web_display: wonton_protocol::TabDisplay::default(),
        cross_origin_isolated: false,
        declares_tools: false,
        tabs: Vec::new(),
        bot_user_id: None,
        family: false,
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
        written_by: None,
        author: None,
        permissions: vec![PermissionLine {
            permission: "storage.share.photos:read".to_owned(),
            text: "读共享文件夹 photos".to_owned(),
            risk: None,
            notice: None,
            supported: true,
            added: false,
        }],
        notices: Vec::new(),
        upgrade_from: None,
        release_notes: None,
        missing_profile: None,
        blocked: None,
        devices: None,
        lan_webui: None,
        always_allowed: Vec::new(),
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
        unreadable: Vec::new(),
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
    pub fn data_dir(&self) -> &Path {
        self._dir.path()
    }

    pub async fn wait_onboarding(
        &self,
        mut done: impl FnMut(&OnboardingViewModel) -> bool,
    ) -> OnboardingViewModel {
        let deadline = Instant::now() + WAIT;
        loop {
            let view = self.app.onboarding();
            if done(&view) {
                return view;
            }
            assert!(Instant::now() < deadline, "{view:?}");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
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

    #[allow(dead_code)]
    pub fn unreachable_ticket(&self) -> String {
        ticket(
            self._home.server.endpoint().id(),
            SocketAddr::from((Ipv4Addr::LOCALHOST, 9)),
        )
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
