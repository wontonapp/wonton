//! 测试里的家：一次性的 Tuwunel，加一个模拟的核心（签发 Matrix 登录令牌，把 `matrix.<基础域名>` 转给 Tuwunel），
//! 前面接真实的隧道服务端。手机是真实的客户端核心，经隧道和本地代理访问，和 App 一样。

use std::collections::{HashMap, VecDeque};
use std::net::SocketAddr;
use std::ops::Deref;
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::stale_join::StaleJoin;
use iroh::{EndpointId, SecretKey};
use serde_json::Value;
use tokio::sync::watch;
use tokio::task::JoinHandle;
use tracing_subscriber::Layer as _;
use tracing_subscriber::filter::filter_fn;
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::util::SubscriberInitExt as _;
use wonton_client::{
    Action, ChatAccount, ChatSync, ChatViewModel, ClientCore, InstalledPlugin, InstalledPlugins,
    Network, OnboardingViewModel, PushMethod,
};
use wonton_matrix::MatrixSession;
pub use wonton_matrix::testkit::ToDeviceHold;
use wonton_matrix::testkit::{Issuer, PushGateway, TestServer, random_hex};
use wonton_tunnel::pair::{
    PairHandler, PairProtocol, PairResponse, PairingCode, PairingSecret, SECRET_LEN,
};
use wonton_tunnel::{Allowlist, TunnelServer};

mod confirmations;
mod diagnostics;
mod endpoint;
mod fake_core;
mod files;
mod friends;
mod io;
mod link_preview;
mod matrix;
mod plugin_start;

pub use diagnostics::{is_room_list, pending_requests, request_arrived, request_done};
use endpoint::loopback_endpoint;
use fake_core::fake_core;
pub(super) use io::{read_body, relay};

pub const WAIT: Duration = Duration::from_secs(60);
pub(super) const BASE_DOMAIN: &str = "wonton.localhost";
/// 和 deploy/dev/matrix/tuwunel.toml 的 server_name 一致。
pub const OWNER: &str = "@owner:wonton.localhost";

/// `WONTON_TEST_LOG` 有值时打开核心的日志（值是过滤规则，例如 `wonton_tunnel=debug,matrix_sdk=debug`），
/// 写到标准错误；失败的时候按时间顺序留下当时发生了什么。
///
/// 不论开不开日志都装一层捕获：时间线重建的原因按房间记下来（见 [`take_rebuild_reason`]），断言失败时带进
/// panic 消息，否则只看到“被清空了”、不知道是哪条路。
pub fn init_test_logging() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let subscriber =
            tracing_subscriber::registry().with(super::rebuild_reason::Capture.with_filter(
                filter_fn(|m| super::rebuild_reason::TARGETS.contains(&m.target())),
            ));
        match std::env::var("WONTON_TEST_LOG").or_else(|_| Ok::<_, ()>("info".to_owned())) {
            Ok(filter) => {
                let filter = tracing_subscriber::EnvFilter::try_new(&filter)
                    .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn"));
                subscriber
                    .with(
                        tracing_subscriber::fmt::layer()
                            .with_ansi(false)
                            .with_writer(std::io::stderr)
                            .with_filter(filter),
                    )
                    .try_init()
                    .ok();
            }
            Err(_) => {
                subscriber.try_init().ok();
            }
        };
    });
}

pub use super::rebuild_reason::take_rebuild_reason;

pub struct Home {
    pub matrix: TestServer,
    pub bot_api: Arc<super::bot_profile_support::BotApi>,
    /// 扮演核心的推送代理：Tuwunel 经出站代理把通知发到这里，和产品里一样。
    pub push: PushGateway,
    core: Arc<Core>,
    tunnel: Option<TunnelServer>,
    tunnel_key: SecretKey,
    tunnel_port: u16,
    pairing: Arc<Pairing>,
    relay: JoinHandle<()>,
    core_addr: SocketAddr,
    _core: JoinHandle<()>,
}

/// 配对的假服务端：配上就把设备放行。chat 的测试只配一次，不区分发出去的码。
#[derive(Debug)]
struct Pairing {
    allowlist: Allowlist,
}

impl PairHandler for Pairing {
    fn pair(
        &self,
        device: EndpointId,
        _secret: &PairingSecret,
        _device_name: &str,
    ) -> PairResponse {
        self.allowlist.allow(device);
        PairResponse::Paired
    }
}

/// 模拟的核心记下的东西。
/// 假核心记着的确认卡片。测试直接塞核心的卡片 JSON，字段同 `ConfirmationCard`。
#[derive(Default, Clone)]
pub(super) struct FakeConfirmations {
    pub(super) pending: Vec<Value>,
    pub(super) finished: Vec<Value>,
    pub(super) contact_requests: Vec<Value>,
    pub(super) revision: u64,
}

struct Core {
    tuwunel: String,
    issuer: Issuer,
    login_tokens: AtomicUsize,
    /// 接下来这么多次要登录令牌的请求回 502，像本地代理那一次没连上隧道。
    failing_login_tokens: AtomicUsize,
    /// 核心的网关连不上 Tuwunel：Matrix 请求都回网关的 503。
    matrix_unavailable: AtomicBool,
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
    /// 为真时推送登记接口的响应先不回（请求照记），让后台登记停在“PUT 已收到、还没登记完”。
    hold_push_registrations: watch::Sender<bool>,
    /// 推送登记请求的到达次数；等的一方靠它的事件变化再查，不睡固定间隔。
    push_registrations_seen: watch::Sender<u64>,
    uploads_paused: watch::Sender<bool>,
    /// 为真时手机的 keys/upload 一接上就断开（drop 语义），见 `chat::key_upload_order` 回归测试。
    hold_key_uploads: Arc<AtomicBool>,
    key_upload_delay: Mutex<Duration>,
    /// Tuwunel 成功应答过的手机 keys/upload 次数；收到完成事件后，设备查询才有因果上的保证。
    key_uploads_completed: watch::Sender<u64>,
    /// 等待函数进门标记：为真表示测试开始等设备密钥上服务器（先于 Bot 发消息就该放行）。
    device_keys_wait_started: watch::Sender<bool>,
    /// 为真时 Tuwunel 像挂了一样：转发中的请求断开，新请求一接上就断开。
    matrix_down: watch::Sender<bool>,
    /// 为真时发消息的请求回 502：核心连不上 Tuwunel。matrix-sdk 会自己重试很久。
    fail_sends: watch::Sender<bool>,
    /// 为真时取房间成员的请求一接上就断开，别的请求不受影响。
    members_down: AtomicBool,
    members_requests_dropped: AtomicUsize,
    /// 为真时取聊天显示方式账号数据的请求一接上就断开，别的请求不受影响。
    layout_down: AtomicBool,
    reject_backup_creation: AtomicBool,
    /// 请求行（小写）含其中一段的 Matrix 请求由假核心直接回这个状态码和错误码，不转给 Tuwunel。
    refusals: Mutex<Vec<matrix::Refusal>>,
    /// 同步应答保持错标直到测试显式清除，避免其他同步连接提前把状态覆盖回来。
    stale_join: watch::Sender<Option<StaleJoin>>,
    join_reported_as_invited: AtomicUsize,
    /// 为真时房间列表的下一个同步请求回 `M_UNKNOWN_POS`，像服务器丢了这条连接的位置。
    expire_room_list: AtomicBool,
    verification_failure: AtomicUsize,
    hold_backup_read: watch::Sender<bool>,
    /// 有值时，发给这台设备（Matrix 设备号）的 `m.key.verification.done` 等新手机的 `m.secret.request`
    /// 转发出去、再过三秒才转发，见 [`Home::deliver_key_request_before_done`]。
    done_after_key_request: Mutex<Option<String>>,
    /// 转发过的 `m.secret.request` 次数。
    key_requests_forwarded: watch::Sender<u64>,
    /// 为真时 `m.secret.request` 由假核心先回 200、留在手里，变回假再转发，像旧设备掉线时排着的请求。
    hold_key_requests: watch::Sender<bool>,
    drop_key_requests: AtomicBool,
    sync_requests: Mutex<Vec<Value>>,
    hold_sync_recording: AtomicBool,
    sync_recording_held: watch::Sender<Option<Value>>,
    sync_recording_released: watch::Sender<bool>,
    /// 引导状态：没有就是没有这个接口（404）。
    onboarding: Mutex<Option<Value>>,
    /// “添加设备”发的配对码（`POST /_wonton/api/pairing-codes`），隧道起来后填上；每次要码换一个新 secret。
    issued_pairing_code: Mutex<Option<String>>,
    /// 发过多少个配对码，用它在每次回应里换一个新 secret。
    pairing_codes_issued: AtomicUsize,
    /// 已装插件的应答：没有就是没有这个接口（404）。
    installed: Mutex<Option<Value>>,
    /// 取已装列表的请求次数。
    installed_requests: AtomicUsize,
    plugin_starts: plugin_start::Starts,
    /// “等你确认”列表的假数据（ADR-0016）：卡片和 revision 由测试改，改了挂着的取列表请求就回来。
    confirmations: Mutex<FakeConfirmations>,
    /// [`Self::confirmations`] 的 revision，等的一方靠它的事件变化再查。
    confirmations_revision: watch::Sender<u64>,
    /// 接下来这么多次取列表的请求回 502，像隧道那一次没连上。
    failing_confirmations: AtomicUsize,
    /// 取列表的请求次数。
    confirmations_requests: AtomicUsize,
    /// 为真时单张卡片接口回 404，像卡片已经结束太久或核心重启过。
    single_confirmation_missing: AtomicBool,
    /// 接下来这么多次单张卡片接口回 503，像核心忙不过来。
    failing_single_confirmation: AtomicUsize,
    /// 单张卡片接口的请求次数。
    single_confirmation_requests: AtomicUsize,
    bot_api: Arc<super::bot_profile_support::BotApi>,
    /// 好友接口的假名单（ADR-0051）。
    friends: Mutex<friends::FakeFriends>,
    /// 文件服务的 tus 上传：编号和内容，传完发链接的测试用。
    files: Mutex<HashMap<String, FakeFile>>,
    file_grants: Mutex<Vec<(String, String)>>,
    next_file: AtomicU64,
    /// 链接预览的假应答：网址 → 状态码和响应体（ADR-0065）。没配的网址回 404。
    link_previews: Mutex<HashMap<String, (u16, String)>>,
    /// 链接预览的图片：id → 类型和字节。
    link_preview_images: Mutex<HashMap<String, (String, Vec<u8>)>>,
    /// 取预览的请求到了几次（图片的请求不算）。
    link_preview_requests: AtomicUsize,
}

/// 假核心记下的一个 tus 上传。
#[derive(Default)]
pub(crate) struct FakeFile {
    pub length: u64,
    pub bytes: Vec<u8>,
    pub name: String,
}

impl Home {
    pub async fn start(devices: &[EndpointId]) -> Self {
        init_test_logging();
        let push = PushGateway::start().await;
        let matrix = TestServer::with_push_gateway(&push).await;
        let core = Arc::new(Core {
            tuwunel: matrix.url.trim_start_matches("http://").to_owned(),
            issuer: matrix.issuer.clone(),
            login_tokens: AtomicUsize::new(0),
            failing_login_tokens: AtomicUsize::new(0),
            matrix_unavailable: AtomicBool::new(false),
            pushkey: random_hex(32),
            push_requests: Mutex::default(),
            answers: Mutex::default(),
            window_stops: Mutex::default(),
            answer_replies: Mutex::default(),
            hold_answers: watch::Sender::new(false),
            hold_push_registrations: watch::Sender::new(false),
            push_registrations_seen: watch::Sender::new(0),
            uploads_paused: watch::Sender::new(false),
            hold_key_uploads: Arc::new(AtomicBool::new(false)),
            key_upload_delay: Mutex::new(Duration::ZERO),
            key_uploads_completed: watch::Sender::new(0),
            device_keys_wait_started: watch::Sender::new(false),
            matrix_down: watch::Sender::new(false),
            fail_sends: watch::Sender::new(false),
            members_down: AtomicBool::new(false),
            members_requests_dropped: AtomicUsize::new(0),
            layout_down: AtomicBool::new(false),
            reject_backup_creation: AtomicBool::new(false),
            refusals: Mutex::default(),
            stale_join: watch::Sender::new(None),
            join_reported_as_invited: AtomicUsize::new(0),
            expire_room_list: AtomicBool::new(false),
            verification_failure: AtomicUsize::new(0),
            hold_backup_read: watch::Sender::new(false),
            done_after_key_request: Mutex::default(),
            key_requests_forwarded: watch::Sender::new(0),
            hold_key_requests: watch::Sender::new(false),
            drop_key_requests: AtomicBool::new(false),
            sync_requests: Mutex::default(),
            hold_sync_recording: AtomicBool::new(false),
            sync_recording_held: watch::Sender::new(None),
            sync_recording_released: watch::Sender::new(false),
            onboarding: Mutex::default(),
            issued_pairing_code: Mutex::default(),
            pairing_codes_issued: AtomicUsize::new(0),
            installed: Mutex::default(),
            installed_requests: AtomicUsize::new(0),
            plugin_starts: plugin_start::Starts::default(),
            confirmations: Mutex::default(),
            confirmations_revision: watch::Sender::new(0),
            failing_confirmations: AtomicUsize::new(0),
            confirmations_requests: AtomicUsize::new(0),
            single_confirmation_missing: AtomicBool::new(false),
            failing_single_confirmation: AtomicUsize::new(0),
            single_confirmation_requests: AtomicUsize::new(0),
            bot_api: Arc::default(),
            friends: Mutex::default(),
            files: Mutex::default(),
            file_grants: Mutex::default(),
            next_file: AtomicU64::new(0),
            link_previews: Mutex::default(),
            link_preview_images: Mutex::default(),
            link_preview_requests: AtomicUsize::new(0),
        });
        let (addr, task) = fake_core(core.clone()).await;
        let tunnel_key = SecretKey::generate();
        let endpoint = loopback_endpoint(tunnel_key.clone(), 0).await;
        let tunnel_port = endpoint.bound_sockets()[0].port();
        let allowlist = Allowlist::new(devices.iter().copied());
        let pairing = Arc::new(Pairing {
            allowlist: allowlist.clone(),
        });
        let (tunnel, listener) = TunnelServer::spawn_embedded_with_pairing(
            endpoint,
            allowlist.clone(),
            PairProtocol::new(pairing.clone()),
        );
        let relay = relay(listener, addr);
        let home = Self {
            matrix,
            push,
            bot_api: core.bot_api.clone(),
            core,
            tunnel: Some(tunnel),
            tunnel_key,
            tunnel_port,
            pairing,
            relay,
            core_addr: addr,
            _core: task,
        };
        *home.core.issued_pairing_code.lock().unwrap() = Some(home.pairing_code());
        home
    }

    /// Tuwunel 挂了或恢复了，隧道不受影响。
    pub fn set_matrix_down(&self, down: bool) {
        self.core.matrix_down.send_replace(down);
    }

    /// 发消息的请求回 502：核心连不上 Tuwunel。matrix-sdk 会自己重试很久，客户端这边不设期限就一直挂着。
    pub fn fail_sends(&self, fail: bool) {
        self.core.fail_sends.send_replace(fail);
    }

    /// 请求行（小写，如 `post /_matrix/client/v3/createroom`）含 `request` 的 Matrix 请求回 `status` 和 Matrix 错误码
    /// `errcode`，像 Tuwunel 拒绝了它。
    pub fn refuse_matrix(&self, request: &str, status: u16, errcode: &str) {
        self.core.refusals.lock().unwrap().push(matrix::Refusal {
            request: request.to_owned(),
            status,
            errcode: errcode.to_owned(),
        });
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

    pub fn fail_verification(&self, status: u16) {
        self.core
            .verification_failure
            .store(status as usize, Ordering::SeqCst);
    }

    pub fn hold_backup_read(&self, hold: bool) {
        self.core.hold_backup_read.send_replace(hold);
    }

    /// 让新手机要钥匙的请求比它发给 `device_id` 的确认完成消息早一轮同步到那台设备：
    /// 新手机先确认、旧手机后确认时会自然出现的顺序（ADR-0046）。
    pub fn deliver_key_request_before_done(&self, device_id: &str) {
        self.core.key_requests_forwarded.send_replace(0);
        *self.core.done_after_key_request.lock().unwrap() = Some(device_id.to_owned());
    }

    /// 要秘密的请求先不送到旧设备：`false` 时把留着的按原顺序补发。
    pub fn hold_key_requests(&self, hold: bool) {
        self.core.hold_key_requests.send_replace(hold);
    }

    pub fn drop_key_requests(&self, drop: bool) {
        self.core.drop_key_requests.store(drop, Ordering::SeqCst);
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

    /// 配一个网址的预览应答：`status` 和核心会回的 `body`（成功是预览 JSON，失败是错误 JSON）。
    pub fn set_link_preview(&self, url: &str, status: u16, body: Value) {
        self.core
            .link_previews
            .lock()
            .unwrap()
            .insert(url.to_owned(), (status, body.to_string()));
    }

    /// 配一张预览图片：按 id 给回类型和字节。
    pub fn set_link_preview_image(&self, id: &str, mime_type: &str, bytes: &[u8]) {
        self.core
            .link_preview_images
            .lock()
            .unwrap()
            .insert(id.to_owned(), (mime_type.to_owned(), bytes.to_vec()));
    }

    /// 取预览的请求到了几次；图片的请求不算，断言“没向核心请求”用它。
    pub fn link_preview_requests(&self) -> usize {
        self.core.link_preview_requests.load(Ordering::SeqCst)
    }

    /// 取房间成员的请求断开或恢复。
    pub fn set_members_down(&self, down: bool) {
        self.core.members_down.store(down, Ordering::SeqCst);
    }

    /// 取聊天显示方式账号数据的请求断开或恢复，别的请求不受影响。
    pub fn set_layout_down(&self, down: bool) {
        self.core.layout_down.store(down, Ordering::SeqCst);
    }

    /// 下一个带着 `stale` 房间加入事件的同步应答起，每个这样的应答都被改写成还是受邀（同步重启后 SDK 会
    /// 重发房间状态，会话里多条连接都会带回它），直到 [`Self::clear_join_report_as_invited`] 清除。
    pub fn report_join_as_invited(&self, stale: StaleJoin) {
        self.core.stale_join.send_replace(Some(stale));
    }

    /// 停止改写：之后的同步应答照原样转发，房间回到真实状态。
    pub fn clear_join_report_as_invited(&self) {
        self.core.stale_join.send_replace(None);
    }

    /// [`Self::report_join_as_invited`] 有没有改写到应答。
    pub fn join_was_reported_as_invited(&self) -> bool {
        self.core.join_reported_as_invited.load(Ordering::SeqCst) > 0
    }

    /// 记下的同步请求体，按客户端进入本地代理的顺序；独立隧道流的到达顺序不代表发送顺序。
    pub fn sync_requests(&self) -> Vec<Value> {
        let mut requests = self.core.sync_requests.lock().unwrap().clone();
        requests.sort_by_key(|request| request["test_sequence"].as_u64().unwrap());
        requests
    }

    pub fn hold_next_unsubscribed_sync_recording(&self) {
        self.core.sync_recording_held.send_replace(None);
        self.core.sync_recording_released.send_replace(false);
        self.core.hold_sync_recording.store(true, Ordering::SeqCst);
    }

    pub async fn held_sync_request(&self) -> Value {
        let mut held = self.core.sync_recording_held.subscribe();
        tokio::time::timeout(WAIT, held.wait_for(Option::is_some))
            .await
            .expect("sync request must reach recording barrier")
            .expect("recording barrier must remain open")
            .clone()
            .unwrap()
    }

    pub fn release_sync_recording(&self) {
        self.core.sync_recording_released.send_replace(true);
    }

    pub fn sync_arrivals(&self) -> Vec<Value> {
        self.core.sync_requests.lock().unwrap().clone()
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
        let endpoint = loopback_endpoint(self.tunnel_key.clone(), self.tunnel_port).await;
        let (tunnel, listener) = TunnelServer::spawn_embedded_with_pairing(
            endpoint,
            self.pairing.allowlist.clone(),
            PairProtocol::new(self.pairing.clone()),
        );
        self.tunnel = Some(tunnel);
        self.relay = relay(listener, self.core_addr);
    }

    /// 同样的身份换一个端口，手机看来就是这台服务器换了地址。
    pub async fn move_tunnel(&mut self) {
        self.stop_tunnel().await;
        let endpoint = loopback_endpoint(self.tunnel_key.clone(), 0).await;
        self.tunnel_port = endpoint.bound_sockets()[0].port();
        let (tunnel, listener) = TunnelServer::spawn_embedded_with_pairing(
            endpoint,
            self.pairing.allowlist.clone(),
            PairProtocol::new(self.pairing.clone()),
        );
        self.tunnel = Some(tunnel);
        self.relay = relay(listener, self.core_addr);
    }

    /// 登记推送时核心回给设备的 pushkey。
    pub fn pushkey(&self) -> &str {
        &self.core.pushkey
    }

    /// 推送登记接口收到的请求。
    pub fn push_requests(&self) -> Vec<(String, Value)> {
        self.core.push_requests.lock().unwrap().clone()
    }

    /// 一个配对码，扫了会和这台服务器配对。
    pub fn pairing_code(&self) -> String {
        let tunnel = self.tunnel.as_ref().expect("隧道没在运行");
        let secret = PairingSecret::from_bytes([7; SECRET_LEN]);
        PairingCode {
            server_id: tunnel.endpoint().id(),
            relays: Vec::new(),
            direct_addrs: tunnel.endpoint().bound_sockets(),
            base_domain: BASE_DOMAIN.to_owned(),
            core_url: url::Url::parse(&format!("http://{BASE_DOMAIN}/")).unwrap(),
            secret,
        }
        .to_uri()
    }

    /// 挂起（`true`）或放行（`false`）推送登记接口的响应；挂起期间请求照记不误。
    pub fn hold_push_registrations(&self, hold: bool) {
        self.core.hold_push_registrations.send_replace(hold);
    }

    /// 订阅推送登记请求的到达次数；等的一方靠事件变化再查，不睡固定间隔。
    pub fn push_registrations_seen(&self) -> watch::Receiver<u64> {
        self.core.push_registrations_seen.subscribe()
    }

    /// 好友接口收到的请求：方法和小写路径。
    pub fn friends_calls(&self) -> Vec<(String, String)> {
        self.core.friends.lock().unwrap().calls.clone()
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

    /// 接下来 `count` 次要登录令牌都回 502，隧道照常连着。
    pub fn fail_login_tokens(&self, count: usize) {
        self.core
            .failing_login_tokens
            .store(count, Ordering::SeqCst);
    }

    /// Tuwunel 在重启：核心的网关回 503（Matrix JSON），和 [`Self::set_matrix_down`] 的断连接不同，隧道和令牌照常。
    pub fn set_matrix_unavailable(&self, unavailable: bool) {
        self.core
            .matrix_unavailable
            .store(unavailable, Ordering::SeqCst);
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

    pub fn delay_key_uploads(&self, delay: Duration) {
        *self.core.key_upload_delay.lock().unwrap() = delay;
    }

    /// 开关本体给放开挂起的任务：先到先放的逻辑里，放行的一方不持有 [`Home`]。
    pub fn key_uploads_hold(&self) -> Arc<AtomicBool> {
        self.core.hold_key_uploads.clone()
    }

    /// 完成计数的接收端：`changed()` 等下一次 keys/upload 收到 Tuwunel 的成功响应。
    pub fn key_uploads_completed(&self) -> watch::Receiver<u64> {
        self.core.key_uploads_completed.subscribe()
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

    /// 同 [`Self::bot`]，但登录走 `homeserver_url`（比如 [`ToDeviceHold`] 的中转），之后的请求都走它。
    pub async fn bot_via(
        &self,
        homeserver_url: &str,
        localpart: &str,
        display_name: &str,
    ) -> MatrixSession {
        let bot = self
            .matrix
            .login_via(
                homeserver_url,
                localpart,
                display_name,
                wonton_matrix::StoreConfig::InMemory,
            )
            .await;
        bot.set_display_name(display_name)
            .await
            .expect("Bot 设置显示名");
        bot.start_sync().await.expect("Bot 开始同步");
        bot
    }
}

/// 测试里的手机：一个客户端核心，连同它的数据目录。
pub struct Phone {
    pub(crate) core: Arc<ClientCore>,
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
        check: impl FnMut(&ChatViewModel) -> Option<R>,
    ) -> R {
        self.wait_chat_within(what, WAIT, check).await
    }
    pub async fn wait_chat_within<R>(
        &self,
        what: &str,
        within: Duration,
        mut check: impl FnMut(&ChatViewModel) -> Option<R>,
    ) -> R {
        let subscription = self.subscribe_chat();
        let since = std::time::Instant::now();
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
        match tokio::time::timeout(within, waiting).await {
            Ok(found) => found,
            Err(_) => {
                let view = self.chat();
                // 起步现场的记录按 Matrix 设备号分开，设备号只在登录后才有。
                let device = match &view.account {
                    ChatAccount::SignedIn { device_id, .. } => device_id.as_str(),
                    _ => "",
                };
                panic!(
                    "{what}：{within:?} 内没有等到，当前：{view:#?}\n同步起步：{}\n挂着的请求：{}",
                    wonton_matrix::last_sync_diagnostic(device),
                    pending_requests(since)
                )
            }
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

pub use super::timeline::Timeline;
