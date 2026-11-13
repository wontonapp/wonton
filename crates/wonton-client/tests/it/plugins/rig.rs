//! 假扮的核心管理接口和驱动 App 的工具，只走回环地址。

use std::collections::{HashMap, VecDeque};
use std::net::{Ipv4Addr, SocketAddr};
use std::path::PathBuf;
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
/// 兜底预算，不是节奏：繁忙时建连实测 16s+（登录令牌两次超时重连），单次建连限时 12s，
/// 失败后按退避重试；取 60s，超过就是真的连不上。
pub const CONNECT_BACKSTOP: Duration = Duration::from_secs(60);
pub const IMAGE: &str = "/_wonton/api/images/abc/icon-192.png";

/// 核心对一个请求的预设回应，排在队列里，用完后回默认的。
#[derive(Clone)]
pub struct Canned(pub u16, pub String);

/// 带响应头的预设回应：状态码、响应头（按顺序）、响应体。
#[derive(Clone)]
pub struct Headered(pub u16, pub Vec<(String, String)>, pub String);

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

/// 排队给一个方法+路径的带延迟预设回应。
type DelayedCanned = VecDeque<(Canned, Duration)>;

#[derive(Default)]
pub struct CoreState {
    /// 依次给出的已安装列表，最后一份一直用下去。
    lists: Mutex<VecDeque<Vec<InstalledPlugin>>>,
    /// 假核心保存的引导状态和服务商列表。
    onboarding: Mutex<Option<serde_json::Value>>,
    providers: Mutex<Option<serde_json::Value>>,
    /// 假核心保存的资料；为空时是还没填的。
    owner: Mutex<Option<serde_json::Value>>,
    /// 某个方法和路径依次给出的回应：App 还会向核心要 Matrix 登录令牌，预设的回应不能让别的请求用掉。
    canned: Mutex<HashMap<(Method, String), VecDeque<Canned>>>,
    /// 用完预设的回应后，某个方法和路径一直用的回应。
    standing: Mutex<HashMap<(Method, String), Canned>>,
    /// 同上，但带响应头：有的接口把错误类别写在响应头里（如 `X-Wonton-Error`）。
    canned_headered: Mutex<HashMap<(Method, String), Headered>>,
    standing_headered: Mutex<HashMap<(Method, String), Headered>>,
    /// 按住不回应的路径：测试要卡住一次请求时（如保存 key 途中再存一次），放行后才回。
    holds: Holds,
    /// 带延迟的预设回应：发出早的请求把它取走，睡完才回，制造“发得早、回得晚”。
    canned_delayed: Mutex<HashMap<(Method, String), DelayedCanned>>,
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
            .entry((method, path.to_owned()))
            .or_default()
            .push_back(canned);
    }

    pub fn answer_always(&self, method: Method, path: &str, canned: Canned) {
        self.standing
            .lock()
            .unwrap()
            .insert((method, path.to_owned()), canned);
    }

    pub fn answer_headered(&self, method: Method, path: &str, headered: Headered) {
        self.canned_headered
            .lock()
            .unwrap()
            .insert((method, path.to_owned()), headered);
    }

    pub fn answer_always_headered(&self, method: Method, path: &str, headered: Headered) {
        self.standing_headered
            .lock()
            .unwrap()
            .insert((method, path.to_owned()), headered);
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

    /// 发得早的请求取走这份回应、睡完 `delay` 才回：制造“发得早、回得晚”的旧响应。
    pub fn answer_delayed(&self, method: Method, path: &str, canned: Canned, delay: Duration) {
        self.canned_delayed
            .lock()
            .unwrap()
            .entry((method, path.to_owned()))
            .or_default()
            .push_back((canned, delay));
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

/// 带响应头的预设回应（如 `X-Wonton-Error`）：排在前头，先于不带头的查。
fn headered_response(core: &CoreState, method: &Method, path: &str) -> Option<Response> {
    let headered = core
        .canned_headered
        .lock()
        .unwrap()
        .remove(&(method.clone(), path.to_owned()))
        .or_else(|| {
            core.standing_headered
                .lock()
                .unwrap()
                .get(&(method.clone(), path.to_owned()))
                .cloned()
        })?;
    let Headered(status, headers, body) = headered;
    let mut builder = Response::builder().status(StatusCode::from_u16(status).unwrap());
    for (name, value) in headers {
        builder = builder.header(name, value);
    }
    Some(builder.body(Body::from(body)).unwrap())
}

/// 默认没有 Bot 钉页；单独要钉页的测试自己在前面拦。
fn empty_pins() -> Response {
    (
        StatusCode::OK,
        serde_json::json!({"pins": [], "consents": []}).to_string(),
    )
        .into_response()
}

/// 带延迟的预设回应：弹出就睡，保证这份旧回应一定是发得早的那个请求取走、晚于新回应到达。
async fn delayed_canned_response(
    core: &CoreState,
    method: &Method,
    path: &str,
) -> Option<Response> {
    let delayed = core
        .canned_delayed
        .lock()
        .unwrap()
        .get_mut(&(method.clone(), path.to_owned()))
        .and_then(|queue| queue.pop_front());
    let (Canned(status, body), delay) = delayed?;
    tokio::time::sleep(delay).await;
    Some(
        Response::builder()
            .status(StatusCode::from_u16(status).unwrap())
            .body(Body::from(body))
            .unwrap(),
    )
}

/// GET /_wonton/api/plugins：依次给出预设的列表，最后一份一直用下去。
fn installed_response(core: &CoreState) -> Response {
    let mut lists = core.lists.lock().unwrap();
    let plugins = if lists.len() > 1 {
        lists.pop_front().unwrap()
    } else {
        lists.front().cloned().unwrap_or_default()
    };
    (
        StatusCode::OK,
        serde_json::to_string(&InstalledPlugins {
            plugins,
            upgrade_check: None,
        })
        .unwrap(),
    )
        .into_response()
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
    if let Some(response) = headered_response(&core, &parts.method, parts.uri.path()) {
        return response;
    }
    if let Some(response) = delayed_canned_response(&core, &parts.method, parts.uri.path()).await {
        return response;
    }
    let canned = core
        .canned
        .lock()
        .unwrap()
        .get_mut(&(parts.method.clone(), parts.uri.path().to_owned()))
        .and_then(|queue| queue.pop_front())
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
        (&Method::GET, "/_wonton/api/plugins") => installed_response(&core),
        (&Method::GET, "/_wonton/api/store") => json(serde_json::to_string(&listing()).unwrap()),
        (&Method::GET, "/_wonton/api/panel/pins") => empty_pins(),
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
    if path == wonton_protocol::ME_PATH {
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
    serde_json::json!({
        "user_id": "@owner:wonton.localhost",
        "username": "owner",
        "is_owner": true,
        "display_name": name,
        "email": email,
    })
}

pub fn installed(id: &str, status: PluginState) -> InstalledPlugin {
    InstalledPlugin {
        chat: Default::default(),
        id: id.to_owned(),
        title: format!("标题 {id}"),
        summary: String::new(),
        version: "1.0.0".to_owned(),
        revision: "1.0.0".to_owned(),
        kind: PluginKind::App,
        runtime: Default::default(),
        icon: None,
        trust: PluginTrust::Official,
        source: None,
        provider: None,
        via_index: None,
        written_by: None,
        status,
        web_url: None,
        cross_origin_isolated: false,
        declares_tools: false,
        tabs: Vec::new(),
        bot_user_id: None,
        family: false,
        upgrade: None,
        removable: true,
        notices: Vec::new(),
        enabled_root_permissions: Vec::new(),
        install_progress: None,
        install_failure: None,
        tunnel_ports: Vec::new(),
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
        plan: "opaque-install-plan".to_owned(),
        id: id.to_owned(),
        title: format!("标题 {id}"),
        license: Some("AGPL-3.0-only".into()),
        license_url: None,
        summary: String::new(),
        version: "1.0.0".to_owned(),
        kind: PluginKind::App,
        runtime: Default::default(),
        security_tier: None,
        scope_choices: Vec::new(),
        has_bot: false,
        icon: None,
        trust: PluginTrust::Official,
        provider: None,
        via_index: None,
        written_by: None,
        author: None,
        widget_capabilities: Vec::new(),
        permissions: vec![PermissionLine {
            permission: "storage.share.photos:read".to_owned(),
            text: "读共享文件夹 photos".to_owned(),
            risk: None,
            notice: None,
            supported: true,
            added: false,
            optional: false,
            enabled: false,
            off_loss: None,
            opened: None,
        }],
        secret_inputs: Vec::new(),
        settings: None,
        notices: Vec::new(),
        missing_requires: Vec::new(),
        missing_wants: Vec::new(),
        wanting_plugins: Vec::new(),
        upgrade_from: None,
        release_notes: None,
        missing_profile: None,
        blocked: None,
        blocked_category: None,
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
        license: None,
        license_url: None,
        stars: None,
        installed_version: None,
        security_tier: None,
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

    /// 关掉隧道的定时探测：繁忙时一次卡满时限的探测会断开健康连接、触发整轮重连，
    /// 插件测试的对象不是隧道恢复。初始建连不受影响。
    pub fn without_periodic_checks(self) -> Self {
        self.app.stop_periodic_checks();
        self
    }

    /// 关掉 App 再打开。
    pub fn reopen(&mut self) {
        self.app =
            ClientCore::with_network(self._dir.path().to_str().unwrap(), Network::DirectOnly)
                .unwrap();
    }
    /// 第一台服务器的槽位目录（ADR-0042）。
    pub fn data_dir(&self) -> PathBuf {
        self._dir.path().join("servers/1")
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

    /// 关掉服务端，模拟断网：客户端那边会看到连接断开。
    pub async fn drop_home(&self) {
        self._home.server.endpoint().close().await;
    }

    pub async fn connected(&self) {
        self.connected_within(WAIT).await;
    }

    /// 同 [`Self::connected`]，兜底预算由调用方按被测路径推导。
    pub async fn connected_within(&self, budget: Duration) {
        self.connect();
        let home = self.app.subscribe_home();
        let waiting = async {
            while let Some(view) = home.next().await {
                if view.status == HomeStatus::Connected && view.proxy.is_some() {
                    return;
                }
            }
            panic!("连接订阅提前结束");
        };
        // 等事件，不每 20ms 轮询：忙起来轮询自己也抢 CPU；超时只兜底，报出当时的状态。
        tokio::time::timeout(budget, waiting)
            .await
            .unwrap_or_else(|_| panic!("等待超时，当前：{:?}", self.app.home()));
    }

    pub async fn wait_for(
        &self,
        mut done: impl FnMut(&PluginsViewModel) -> bool,
    ) -> PluginsViewModel {
        let plugins = self.app.subscribe_plugins();
        let waiting = async {
            while let Some(view) = plugins.next().await {
                if done(&view) {
                    return view;
                }
            }
            panic!("订阅提前结束");
        };
        // 等事件，不每 20ms 轮询整份 ViewModel；超时只兜底，报出当时的状态。
        // 闭包只许读这份 ViewModel：读外部状态的只有等 DELETE 落到核心那一条，
        // 而 DELETE 一定先于随后的任务状态发布，事件等得到。
        tokio::time::timeout(WAIT, waiting)
            .await
            .unwrap_or_else(|_| panic!("等待超时，当前：{:?}", self.app.plugins()))
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

    /// 同 [`Self::wait_for`]，兜底预算由调用方按被测路径推导：繁忙时一次请求实测会停滞数秒，
    /// 固定的 20 秒会成为绑定约束而不是兜底。
    pub async fn wait_for_within(
        &self,
        budget: Duration,
        mut done: impl FnMut(&PluginsViewModel) -> bool,
    ) -> PluginsViewModel {
        let plugins = self.app.subscribe_plugins();
        let waiting = async {
            while let Some(view) = plugins.next().await {
                if done(&view) {
                    return view;
                }
            }
            panic!("订阅提前结束");
        };
        tokio::time::timeout(budget, waiting)
            .await
            .unwrap_or_else(|_| panic!("等待超时，当前：{:?}", self.app.plugins()))
    }

    /// 同 [`Self::task`]，兜底预算见 [`Self::wait_for_within`]。
    pub async fn task_within(
        &self,
        id: &str,
        budget: Duration,
        mut done: impl FnMut(&PluginTaskState) -> bool,
    ) -> PluginTaskState {
        let view = self
            .wait_for_within(budget, |view| {
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
