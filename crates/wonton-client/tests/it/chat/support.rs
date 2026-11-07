//! 测试里的家：一次性的 Tuwunel，加一个模拟的核心（签发 Matrix 登录令牌，把 `matrix.<基础域名>` 转给 Tuwunel），
//! 前面接真实的隧道服务端。手机是真实的客户端核心，经隧道和本地代理访问，和 App 一样。

use std::collections::VecDeque;
use std::net::{Ipv4Addr, SocketAddr};
use std::ops::Deref;
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use iroh::endpoint::presets;
use iroh::{Endpoint, EndpointId, SecretKey};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::watch;
use tokio::task::JoinHandle;
use wonton_client::{
    Action, BackPagination, ChatAccount, ChatSync, ChatViewModel, ClientCore, InstalledPlugin,
    InstalledPlugins, Network, OnboardingViewModel, PushMethod, TimelineDiff, TimelineItem,
    TimelineSubscription,
};
use wonton_matrix::MatrixSession;
use wonton_matrix::testkit::{Issuer, PushGateway, TestServer, random_hex};
use wonton_tunnel::{Allowlist, TunnelServer};

use super::stale_join::{StaleJoin, forward_one};

pub const WAIT: Duration = Duration::from_secs(60);
const BASE_DOMAIN: &str = "wonton.localhost";
/// 和 deploy/dev/matrix/tuwunel.toml 的 server_name 一致。
pub const OWNER: &str = "@owner:wonton.localhost";

pub struct Home {
    pub matrix: TestServer,
    pub bot_api: Arc<super::bot_profile_support::BotApi>,
    /// 扮演核心的推送代理：Tuwunel 经出站代理把通知发到这里，和产品里一样。
    pub push: PushGateway,
    core: Arc<Core>,
    tunnel: Option<TunnelServer>,
    tunnel_key: SecretKey,
    tunnel_port: u16,
    allowlist: Allowlist,
    core_addr: SocketAddr,
    _core: JoinHandle<()>,
}

/// 模拟的核心记下的东西。
struct Core {
    tuwunel: String,
    issuer: Issuer,
    login_tokens: AtomicUsize,
    /// 登记推送时回给设备的 pushkey。
    pushkey: String,
    /// 推送登记接口收到的请求：方法和请求体（`DELETE` 没有请求体，是 `Null`）。
    push_requests: Mutex<Vec<(String, Value)>>,
    /// 确认卡片收到的回答：卡片 ID 和请求体。
    answers: Mutex<Vec<(String, Value)>>,
    window_stops: Mutex<Vec<String>>,
    /// 接下来的回答怎么回；排空了就照常收下。
    answer_replies: Mutex<VecDeque<(u16, Value)>>,
    /// 为真时回答先不回，手机上这时是“正在发”。
    hold_answers: watch::Sender<bool>,
    uploads_paused: watch::Sender<bool>,
    /// 为真时手机的 keys/upload 一接上就断开（drop 语义），见 `chat::key_upload_order` 回归测试。
    hold_key_uploads: Arc<AtomicBool>,
    /// 放行过的手机 keys/upload 次数；等“设备密钥上服务器”的一方靠它的事件变化再查，不睡固定间隔。
    key_uploads_forwarded: watch::Sender<u64>,
    /// 等待函数进门标记：为真表示测试开始等设备密钥上服务器（先于 Bot 发消息就该放行）。
    device_keys_wait_started: watch::Sender<bool>,
    /// 为真时 Tuwunel 像挂了一样：转发中的请求断开，新请求一接上就断开。
    matrix_down: watch::Sender<bool>,
    /// 为真时取房间成员的请求一接上就断开，别的请求不受影响。
    members_down: AtomicBool,
    members_requests_dropped: AtomicUsize,
    reject_backup_creation: AtomicBool,
    /// 有值时下一个带着这个房间加入事件的同步应答被改写成“受邀”，见 [`super::stale_join`]。
    stale_join: watch::Sender<Option<StaleJoin>>,
    join_reported_as_invited: AtomicBool,
    /// 为真时房间列表的下一个同步请求回 `M_UNKNOWN_POS`，像服务器丢了这条连接的位置。
    expire_room_list: AtomicBool,
    /// 引导状态：没有就是没有这个接口（404）。
    onboarding: Mutex<Option<Value>>,
    /// 已装插件的应答：没有就是没有这个接口（404）。
    installed: Mutex<Option<Value>>,
    /// 取已装列表的请求次数。
    installed_requests: AtomicUsize,
    bot_api: Arc<super::bot_profile_support::BotApi>,
}

impl Home {
    pub async fn start(devices: &[EndpointId]) -> Self {
        let push = PushGateway::start().await;
        let matrix = TestServer::with_push_gateway(&push).await;
        let core = Arc::new(Core {
            tuwunel: matrix.url.trim_start_matches("http://").to_owned(),
            issuer: matrix.issuer.clone(),
            login_tokens: AtomicUsize::new(0),
            pushkey: random_hex(32),
            push_requests: Mutex::default(),
            answers: Mutex::default(),
            window_stops: Mutex::default(),
            answer_replies: Mutex::default(),
            hold_answers: watch::Sender::new(false),
            uploads_paused: watch::Sender::new(false),
            hold_key_uploads: Arc::new(AtomicBool::new(false)),
            key_uploads_forwarded: watch::Sender::new(0),
            device_keys_wait_started: watch::Sender::new(false),
            matrix_down: watch::Sender::new(false),
            members_down: AtomicBool::new(false),
            members_requests_dropped: AtomicUsize::new(0),
            reject_backup_creation: AtomicBool::new(false),
            stale_join: watch::Sender::new(None),
            join_reported_as_invited: AtomicBool::new(false),
            expire_room_list: AtomicBool::new(false),
            onboarding: Mutex::default(),
            installed: Mutex::default(),
            installed_requests: AtomicUsize::new(0),
            bot_api: Arc::default(),
        });
        let (addr, task) = fake_core(core.clone()).await;
        let tunnel_key = SecretKey::generate();
        let endpoint = loopback_endpoint(tunnel_key.clone(), 0).await;
        let tunnel_port = endpoint.bound_sockets()[0].port();
        let allowlist = Allowlist::new(devices.iter().copied());
        let tunnel = TunnelServer::spawn(endpoint, allowlist.clone(), addr);
        Self {
            matrix,
            push,
            bot_api: core.bot_api.clone(),
            core,
            tunnel: Some(tunnel),
            tunnel_key,
            tunnel_port,
            allowlist,
            core_addr: addr,
            _core: task,
        }
    }

    /// Tuwunel 挂了或恢复了，隧道不受影响。
    pub fn set_matrix_down(&self, down: bool) {
        self.core.matrix_down.send_replace(down);
    }

    /// 服务器丢了房间列表那条 Sliding Sync 连接的位置：下一个请求回 `M_UNKNOWN_POS`，SDK 从头同步。
    pub fn expire_room_list_sync(&self) {
        self.core.expire_room_list.store(true, Ordering::SeqCst);
    }

    /// 上一次 [`Self::expire_room_list_sync`] 已经用掉。
    pub fn room_list_sync_expired(&self) -> bool {
        !self.core.expire_room_list.load(Ordering::SeqCst)
    }

    pub fn reject_backup_creation(&self, reject: bool) {
        self.core
            .reject_backup_creation
            .store(reject, Ordering::SeqCst);
    }

    /// 设置假核心的引导状态；没设置时引导接口回 404。
    pub fn set_onboarding(&self, onboarding: Value) {
        *self.core.onboarding.lock().unwrap() = Some(onboarding);
    }

    /// 设置假核心的已装列表应答；没设置时这个接口回 404。
    pub fn set_installed_plugins(&self, plugins: Vec<InstalledPlugin>) {
        let installed = serde_json::to_value(InstalledPlugins {
            plugins,
            upgrade_check: None,
        })
        .unwrap();
        *self.core.installed.lock().unwrap() = Some(installed);
    }

    /// 取已装列表的请求次数，数的是这个测试会话里的全部。
    pub fn installed_requests(&self) -> usize {
        self.core.installed_requests.load(Ordering::SeqCst)
    }

    /// 取房间成员的请求断开或恢复。
    pub fn set_members_down(&self, down: bool) {
        self.core.members_down.store(down, Ordering::SeqCst);
    }

    /// 下一个带着 `stale` 房间加入事件的同步应答，把本人报成还是受邀。正在转发的请求断开重发，保证经过改写。
    pub fn report_join_as_invited(&self, stale: StaleJoin) {
        self.core.stale_join.send_replace(Some(stale));
    }

    /// [`Self::report_join_as_invited`] 有没有改写到应答。
    pub fn join_was_reported_as_invited(&self) -> bool {
        self.core.join_reported_as_invited.load(Ordering::SeqCst)
    }

    /// 被 [`Self::set_members_down`] 断开过多少次取房间成员的请求。
    pub fn members_requests_dropped(&self) -> usize {
        self.core.members_requests_dropped.load(Ordering::SeqCst)
    }

    /// 家里的隧道断了：手机连不上，Tuwunel 和直连它的 Bot 不受影响。
    /// 隧道的 endpoint ID，推送里写的就是它。
    pub fn server_id(&self) -> String {
        self.tunnel_key.public().to_string()
    }

    pub async fn stop_tunnel(&mut self) {
        self.tunnel.take().expect("隧道没在运行").shutdown().await;
    }

    /// 用同样的身份和端口恢复隧道。
    pub async fn start_tunnel(&mut self) {
        let deadline = tokio::time::Instant::now() + WAIT;
        // iroh 关闭后要再过一小会儿才释放 UDP 端口。
        while std::net::UdpSocket::bind((Ipv4Addr::LOCALHOST, self.tunnel_port)).is_err() {
            assert!(
                tokio::time::Instant::now() < deadline,
                "隧道的端口一直没有释放"
            );
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
        let endpoint = loopback_endpoint(self.tunnel_key.clone(), self.tunnel_port).await;
        self.tunnel = Some(TunnelServer::spawn(
            endpoint,
            self.allowlist.clone(),
            self.core_addr,
        ));
    }

    /// 同样的身份换一个端口，手机看来就是这台服务器换了地址。
    pub async fn move_tunnel(&mut self) {
        self.stop_tunnel().await;
        let endpoint = loopback_endpoint(self.tunnel_key.clone(), 0).await;
        self.tunnel_port = endpoint.bound_sockets()[0].port();
        self.tunnel = Some(TunnelServer::spawn(
            endpoint,
            self.allowlist.clone(),
            self.core_addr,
        ));
    }

    /// 登记推送时核心回给设备的 pushkey。
    pub fn pushkey(&self) -> &str {
        &self.core.pushkey
    }

    /// 推送登记接口收到的请求。
    pub fn push_requests(&self) -> Vec<(String, Value)> {
        self.core.push_requests.lock().unwrap().clone()
    }

    pub fn ticket(&self) -> String {
        let tunnel = self.tunnel.as_ref().expect("隧道没在运行");
        let id = tunnel.endpoint().id();
        let addr = tunnel.endpoint().bound_sockets()[0];
        format!(
            r#"{{"server": {{"endpoint_id": "{id}", "direct_addrs": ["{addr}"]}},
                "base_domain": "{BASE_DOMAIN}", "core_url": "http://{BASE_DOMAIN}"}}"#
        )
    }

    /// 确认卡片收到的回答：卡片 ID 和请求体。
    pub fn confirmation_answers(&self) -> Vec<(String, Value)> {
        self.core.answers.lock().unwrap().clone()
    }

    /// 放行时段收到的“停止”：卡片 ID。
    pub fn window_stops(&self) -> Vec<String> {
        self.core.window_stops.lock().unwrap().clone()
    }

    /// 下一个回答或者“停止”这样回。
    pub fn reply_to_next_answer(&self, status: u16, body: Value) {
        self.core
            .answer_replies
            .lock()
            .unwrap()
            .push_back((status, body));
    }

    pub fn hold_answers(&self, hold: bool) {
        self.core.hold_answers.send_replace(hold);
    }

    /// 核心签发过几次登录令牌。
    pub fn login_tokens(&self) -> usize {
        self.core.login_tokens.load(Ordering::SeqCst)
    }

    /// 停在上传请求体前，避免取消测试和上传完成竞跑。
    pub fn pause_uploads(&self) {
        self.core.uploads_paused.send_replace(true);
    }

    /// 回归测试开关：挂起期间手机的 keys/upload 一接上就断开（drop 语义，不挡其他请求；SDK 的
    /// outgoing 请求失败只记日志，下一轮同步自动重试）。
    pub fn hold_key_uploads(&self) {
        self.core.hold_key_uploads.store(true, Ordering::SeqCst);
    }

    /// 开关本体给放开挂起的任务：先到先放的逻辑里，放行的一方不持有 [`Home`]。
    pub fn key_uploads_hold(&self) -> Arc<AtomicBool> {
        self.core.hold_key_uploads.clone()
    }

    /// 转发计数的接收端：`changed()` 等下一次 keys/upload 被放行。
    pub fn key_uploads_forwarded(&self) -> watch::Receiver<u64> {
        self.core.key_uploads_forwarded.subscribe()
    }

    /// 等待函数进门时调用：放开的任务看到它就知道测试开始等设备密钥了。
    pub fn mark_device_keys_wait_started(&self) {
        self.core.device_keys_wait_started.send_replace(true);
    }

    pub fn device_keys_wait_started(&self) -> watch::Receiver<bool> {
        self.core.device_keys_wait_started.subscribe()
    }

    pub fn resume_uploads(&self) {
        self.core.uploads_paused.send_replace(false);
    }

    /// 一个 Bot 账号：直接连 Tuwunel（Bot 跑在家里的服务器上，不经隧道），设好显示名并开始同步。
    pub async fn bot(&self, localpart: &str, display_name: &str) -> MatrixSession {
        let bot = self.matrix.login(localpart, display_name).await;
        bot.set_display_name(display_name)
            .await
            .expect("Bot 设置显示名");
        bot.start_sync().await.expect("Bot 开始同步");
        bot
    }
}

/// 模拟的核心：`POST /_wonton/matrix/login-token` 签发主人的登录令牌（字段同核心的 `LoginToken`），
/// `PUT`、`DELETE /_wonton/push/registration` 记下推送登记（响应同核心的推送登记接口），`matrix.<基础域名>` 上的
/// 请求原样转给 Tuwunel。本地代理每个请求一条连接，读完请求头就能分流。
async fn fake_core(core: Arc<Core>) -> (SocketAddr, JoinHandle<()>) {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let addr = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        loop {
            let (tcp, _) = listener.accept().await.unwrap();
            tokio::spawn(serve(tcp, core.clone()));
        }
    });
    (addr, task)
}

async fn serve(mut tcp: TcpStream, core: Arc<Core>) {
    let mut head = Vec::new();
    while !head.ends_with(b"\r\n\r\n") {
        let mut byte = [0u8; 1];
        if tcp.read(&mut byte).await.unwrap_or(0) == 0 {
            return;
        }
        head.push(byte[0]);
    }
    let text = String::from_utf8_lossy(&head).to_ascii_lowercase();
    if text.contains(&format!("\r\nhost: matrix.{BASE_DOMAIN}")) {
        core.bot_api.observe_matrix(&text);
        if text.starts_with("post /_matrix/client/v3/room_keys/version ")
            && core.reject_backup_creation.load(Ordering::SeqCst)
        {
            let body = r#"{"errcode":"M_FORBIDDEN","error":"Backup creation rejected"}"#;
            let response = format!(
                "HTTP/1.1 403 Forbidden\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\
                 connection: close\r\n\r\n{body}",
                body.len()
            );
            tcp.write_all(response.as_bytes()).await.unwrap();
            tcp.shutdown().await.unwrap();
            return;
        }
        if text.starts_with("post /_matrix/media/")
            && text
                .lines()
                .next()
                .is_some_and(|line| line.contains("/upload"))
        {
            let mut paused = core.uploads_paused.subscribe();
            paused.wait_for(|paused| !paused).await.unwrap();
        }
        // `chat::key_upload_order` 回归测试：挂起期间手机的 keys/upload 一接上就断开（drop 语义），
        // 放行后才转发并计数；SDK 那头只记一条 outgoing 请求失败的日志，下一轮同步自动重试。
        if text.starts_with("post /_matrix/client/v3/keys/upload") {
            if core.hold_key_uploads.load(Ordering::SeqCst) {
                return;
            }
            core.key_uploads_forwarded.send_modify(|count| *count += 1);
        }
        let mut down = core.matrix_down.subscribe();
        if *down.borrow() {
            return;
        }
        let asks_for_members = text
            .lines()
            .next()
            .is_some_and(|line| line.contains("/members"));
        if asks_for_members && core.members_down.load(Ordering::SeqCst) {
            core.members_requests_dropped.fetch_add(1, Ordering::SeqCst);
            return;
        }
        let mut stale = core.stale_join.subscribe();
        let armed = stale.borrow().clone();
        if let Some(armed) = armed {
            if forward_one(tcp, &core.tuwunel, &head, &armed).await {
                core.stale_join.send_replace(None);
                core.join_reported_as_invited.store(true, Ordering::SeqCst);
            }
            return;
        }
        if expire_room_list(&mut tcp, &core, &text, &mut head).await {
            return;
        }
        let mut upstream = TcpStream::connect(core.tuwunel.as_str()).await.unwrap();
        upstream.write_all(&head).await.unwrap();
        tokio::select! {
            _ = tokio::io::copy_bidirectional(&mut tcp, &mut upstream) => {}
            _ = down.wait_for(|down| *down) => {}
            _ = stale.wait_for(Option::is_some) => {}
        }
        return;
    }
    let Some(body) = read_body(&mut tcp, &text).await else {
        return;
    };
    if text.starts_with("post /_wonton/confirmations/")
        || text.starts_with("delete /_wonton/confirmations/")
    {
        let mut released = core.hold_answers.subscribe();
        released.wait_for(|hold| !hold).await.ok();
    }
    let reply = match core.bot_api.respond(&text, &body).await {
        Some(reply) => Some(reply),
        None => respond(&core, &text, &body),
    };
    let response = match reply {
        Some((status, body)) => format!(
            "HTTP/1.1 {status} Whatever\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\
             connection: close\r\n\r\n{body}",
            body.len()
        ),
        None => {
            "HTTP/1.1 404 Not Found\r\ncontent-length: 0\r\nconnection: close\r\n\r\n".to_owned()
        }
    };
    tcp.write_all(response.as_bytes()).await.ok();
    tcp.shutdown().await.ok();
}

/// 要求过期时，房间列表的下一个同步请求回 `M_UNKNOWN_POS`。连接号在请求体里，所以先读完请求体，别的同步请求连同
/// 请求体照常转发。回真时这个请求已经答完（或者读不下去了）。
async fn expire_room_list(
    tcp: &mut TcpStream,
    core: &Core,
    text: &str,
    head: &mut Vec<u8>,
) -> bool {
    if !text.starts_with("post /_matrix/client/unstable/org.matrix.simplified_msc3575/sync")
        || !core.expire_room_list.load(Ordering::SeqCst)
    {
        return false;
    }
    let Some(body) = read_body(tcp, text).await else {
        return true;
    };
    if String::from_utf8_lossy(&body).contains(r#""conn_id":"room-list""#)
        && core.expire_room_list.swap(false, Ordering::SeqCst)
    {
        let body = r#"{"errcode":"M_UNKNOWN_POS","error":"Unknown position"}"#;
        let response = format!(
            "HTTP/1.1 400 Bad Request\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\
             connection: close\r\n\r\n{body}",
            body.len()
        );
        tcp.write_all(response.as_bytes()).await.ok();
        tcp.shutdown().await.ok();
        return true;
    }
    head.extend_from_slice(&body);
    false
}

async fn read_body(tcp: &mut TcpStream, head: &str) -> Option<Vec<u8>> {
    let length = head
        .lines()
        .find_map(|line| line.strip_prefix("content-length:"))
        .and_then(|value| value.trim().parse().ok())
        .unwrap_or(0);
    let mut body = vec![0u8; length];
    tcp.read_exact(&mut body).await.ok()?;
    Some(body)
}

/// 核心自己的接口；不认识的请求回 `None`（404）。
fn respond(core: &Core, head: &str, body: &[u8]) -> Option<(u16, String)> {
    let push = |method: &str, body: Value| {
        core.push_requests
            .lock()
            .unwrap()
            .push((method.to_owned(), body));
    };
    if let Some(rest) = head.strip_prefix("post /_wonton/confirmations/") {
        let id = rest
            .split_whitespace()
            .next()
            .unwrap_or_default()
            .to_owned();
        let answer: Value = serde_json::from_slice(body).unwrap_or(Value::Null);
        let state = match answer["answer"].as_str() {
            Some("approve") => "running",
            _ => "rejected",
        };
        core.answers.lock().unwrap().push((id, answer));
        let (status, reply) = core
            .answer_replies
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| (200, json!({"state": {"type": state}})));
        return Some((status, reply.to_string()));
    }
    if let Some(rest) = head.strip_prefix("delete /_wonton/confirmations/") {
        let id = rest.split('/').next().unwrap_or_default().to_owned();
        core.window_stops.lock().unwrap().push(id);
        let reply = core.answer_replies.lock().unwrap().pop_front();
        return Some(reply.map_or((204, String::new()), |(status, reply)| {
            (status, reply.to_string())
        }));
    }
    if head.starts_with("get /_wonton/api/plugins ") {
        core.installed_requests.fetch_add(1, Ordering::SeqCst);
        let installed = core.installed.lock().unwrap().clone()?;
        return Some((200, installed.to_string()));
    }
    let response = if head.starts_with("post /_wonton/matrix/login-token ") {
        core.login_tokens.fetch_add(1, Ordering::SeqCst);
        json!({
            "homeserver_url": format!("http://matrix.{BASE_DOMAIN}"),
            "server_name": BASE_DOMAIN,
            "user_id": OWNER,
            "login_type": "org.matrix.login.jwt",
            "token": core.issuer.login_token("owner"),
            "expires_in": 120,
        })
    } else if head.starts_with("put /_wonton/push/registration ") {
        push("PUT", serde_json::from_slice(body).unwrap_or(Value::Null));
        json!({"pushkey": core.pushkey, "gateway_url": PushGateway::URL})
    } else if head.starts_with("get /_wonton/api/onboarding ") {
        let value = core.onboarding.lock().unwrap().clone()?;
        return Some((200, value.to_string()));
    } else if head.starts_with("delete /_wonton/push/registration ") {
        push("DELETE", Value::Null);
        json!({"removed": true})
    } else {
        return None;
    };
    Some((200, response.to_string()))
}

async fn loopback_endpoint(key: SecretKey, port: u16) -> Endpoint {
    Endpoint::builder(presets::Minimal)
        .secret_key(key)
        .clear_ip_transports()
        .bind_addr((Ipv4Addr::LOCALHOST, port))
        .unwrap()
        .bind()
        .await
        .unwrap()
}

/// 测试里的手机：一个客户端核心，连同它的数据目录。
pub struct Phone {
    core: Arc<ClientCore>,
}

impl Deref for Phone {
    type Target = ClientCore;

    fn deref(&self) -> &ClientCore {
        &self.core
    }
}

impl Phone {
    pub fn open(dir: &std::path::Path) -> Self {
        let core = ClientCore::with_network(dir.to_str().unwrap(), Network::DirectOnly).unwrap();
        Self { core }
    }

    pub fn device_id(&self) -> EndpointId {
        EndpointId::from_str(&self.home().device_id).unwrap()
    }

    pub fn connect(&self, home: &Home) {
        self.dispatch(Action::ConnectHome {
            ticket: home.ticket(),
        })
        .unwrap();
    }

    /// 等聊天状态满足条件。
    pub async fn wait_chat<R>(
        &self,
        what: &str,
        mut check: impl FnMut(&ChatViewModel) -> Option<R>,
    ) -> R {
        let subscription = self.subscribe_chat();
        let mut last = None;
        let waiting = async {
            while let Some(view) = subscription.next().await {
                if let Some(found) = check(&view) {
                    return found;
                }
                last = Some(view);
            }
            panic!("订阅提前结束");
        };
        match tokio::time::timeout(WAIT, waiting).await {
            Ok(found) => found,
            Err(_) => panic!("{what}：{WAIT:?} 内没有等到，当前：{:#?}", self.chat()),
        }
    }

    /// 等登录并同步起来，返回用户 ID 和设备 ID。
    pub async fn signed_in(&self) -> (String, String) {
        self.wait_chat("登录并开始同步", |view| {
            match (&view.account, &view.sync) {
                (ChatAccount::SignedIn { user_id, device_id }, ChatSync::Running) => {
                    Some((user_id.clone(), device_id.clone()))
                }
                _ => None,
            }
        })
        .await
    }

    /// 这些测试登记推送端点，走的是 UnifiedPush：默认方式是保持连接（ADR-0003），先切过去才登记得了。
    pub async fn use_unified_push(&self) {
        self.dispatch(Action::SetPushMethod {
            method: PushMethod::UnifiedPush,
        })
        .unwrap();
    }

    /// 等引导的视图满足条件。
    pub async fn wait_onboarding_prompts<R>(
        &self,
        what: &str,
        mut check: impl FnMut(&OnboardingViewModel) -> Option<R>,
    ) -> R {
        let subscription = self.subscribe_onboarding();
        let waiting = async {
            while let Some(view) = subscription.next().await {
                if let Some(found) = check(&view) {
                    return found;
                }
            }
            panic!("订阅提前结束");
        };
        match tokio::time::timeout(WAIT, waiting).await {
            Ok(found) => found,
            Err(_) => panic!(
                "{what}：{WAIT:?} 内没有等到，当前：{:#?}",
                self.onboarding()
            ),
        }
    }
}

/// 壳那一侧维护的时间线：按顺序应用每一批增量。同步从头开始时（服务器回 `M_UNKNOWN_POS`）SDK 只留最新的一段，
/// 列表被清空重来；壳这时露出顶上那一行，自己往回翻，这里也一样。
pub struct Timeline {
    pub items: Vec<TimelineItem>,
    subscription: Arc<TimelineSubscription>,
    core: Arc<ClientCore>,
    room_id: String,
    back_pagination: BackPagination,
    started: bool,
    cleared: bool,
    paging: bool,
}

impl Timeline {
    pub async fn open(phone: &Phone, room_id: &str) -> Self {
        let subscription = phone
            .subscribe_timeline(room_id.to_owned())
            .await
            .expect("订阅时间线");
        Self {
            items: Vec::new(),
            subscription,
            core: phone.core.clone(),
            room_id: room_id.to_owned(),
            back_pagination: BackPagination::Idle,
            started: false,
            cleared: false,
            paging: false,
        }
    }

    /// 应用一批增量。首批之后的 `Clear`、`Reset` 是列表被清空重来。
    fn apply_update(&mut self, diffs: Vec<TimelineDiff>, back_pagination: BackPagination) {
        // 翻页开始了，或者翻回来的条目到了：这一页算翻过，下回还在顶上就再翻。
        if back_pagination != BackPagination::Idle || !diffs.is_empty() {
            self.paging = false;
        }
        for diff in diffs {
            if self.started && matches!(diff, TimelineDiff::Clear | TimelineDiff::Reset { .. }) {
                self.cleared = true;
            }
            apply(&mut self.items, diff);
        }
        self.started = true;
        self.back_pagination = back_pagination;
    }

    pub fn was_cleared(&self) -> bool {
        self.cleared
    }

    /// 清空过、还没翻到头、没在翻：像壳一样再翻一页。
    fn page_back_if_cleared(&mut self) {
        if self.cleared && !self.paging && self.back_pagination == BackPagination::Idle {
            self.paging = true;
            self.core
                .dispatch(Action::PaginateBackwards {
                    room_id: self.room_id.clone(),
                })
                .expect("往回翻");
        }
    }

    /// 应用增量，直到列表里有满足条件的条目。
    pub async fn until<R>(
        &mut self,
        what: &str,
        mut check: impl FnMut(&TimelineItem) -> Option<R>,
    ) -> R {
        self.until_items(what, |items| items.iter().find_map(&mut check))
            .await
    }

    /// 应用增量，直到整个列表满足条件。
    pub async fn until_items<R>(
        &mut self,
        what: &str,
        mut check: impl FnMut(&[TimelineItem]) -> Option<R>,
    ) -> R {
        let waiting = async {
            loop {
                if let Some(found) = check(&self.items) {
                    return found;
                }
                self.page_back_if_cleared();
                let update = self.subscription.next().await.expect("时间线订阅提前结束");
                self.apply_update(update.diffs, update.back_pagination);
            }
        };
        match tokio::time::timeout(WAIT, waiting).await {
            Ok(found) => found,
            Err(_) => panic!("{what}：{WAIT:?} 内没有等到，当前：{:#?}", self.items),
        }
    }
}

fn apply(items: &mut Vec<TimelineItem>, diff: TimelineDiff) {
    match diff {
        TimelineDiff::Append { items: more } => items.extend(more),
        TimelineDiff::Clear => items.clear(),
        TimelineDiff::PushFront { item } => items.insert(0, item),
        TimelineDiff::PushBack { item } => items.push(item),
        TimelineDiff::PopFront => {
            items.remove(0);
        }
        TimelineDiff::PopBack => {
            items.pop();
        }
        TimelineDiff::Insert { index, item } => items.insert(index as usize, item),
        TimelineDiff::Set { index, item } => items[index as usize] = item,
        TimelineDiff::Remove { index } => {
            items.remove(index as usize);
        }
        TimelineDiff::Truncate { length } => items.truncate(length as usize),
        TimelineDiff::Reset { items: all } => *items = all,
    }
}
