//! 测试里的家：一次性的 Tuwunel，加一个模拟的核心（签发 Matrix 登录令牌，把 `matrix.<基础域名>` 转给 Tuwunel），
//! 前面接真实的隧道服务端。手机是真实的客户端核心，经隧道和本地代理访问，和 App 一样。

use std::collections::{HashMap, VecDeque};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::stale_join::StaleJoin;
use iroh::{EndpointId, SecretKey};
use serde_json::Value;
use tokio::sync::watch;
use tokio::task::JoinHandle;
pub use wonton_matrix::testkit::ToDeviceHold;
use wonton_matrix::testkit::{Issuer, PushGateway, TestServer, random_hex};
use wonton_tunnel::pair::PairProtocol;
use wonton_tunnel::{Allowlist, TunnelServer};

mod backup;
mod confirmations;
mod diagnostics;
mod endpoint;
mod fake_core;
mod files;
mod friends;
mod home;
mod io;
mod key_handoff;
mod link_preview;
mod logging;
mod login;
mod matrix;
mod onboarding;
mod phone;
mod plugin_start;
mod plugins;
mod sync;
mod wait;

pub(in crate::chat) use confirmations::FakeConfirmations;

pub use diagnostics::{is_room_list, pending_requests, request_arrived, request_done};
use endpoint::loopback_endpoint;
use fake_core::fake_core;
pub(super) use home::BASE_DOMAIN;
pub use home::OWNER;
use home::Pairing;
pub(super) use io::{read_body, relay};
pub use logging::init_test_logging;
pub use phone::Phone;
pub use wait::WAIT;

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
    /// 本人的私人 Bot 接口应答：没有就是没有这个接口（404）。
    own_private_bot: Mutex<Option<Value>>,
    /// `GET /_wonton/api/me` 的资料，`PUT` 改名字和邮箱。
    me: Mutex<Value>,
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

impl Core {
    fn new(tuwunel: String, issuer: Issuer) -> Self {
        Self {
            tuwunel,
            issuer,
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
            own_private_bot: Mutex::default(),
            me: Mutex::new(serde_json::json!({
                "user_id": OWNER,
                "username": "owner",
                "is_owner": true,
                "role_id": null,
                "role_name": null,
                "permissions": null,
                "display_name": null,
                "email": null,
                "avatar": null,
            })),
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
        }
    }
}

impl Home {
    pub async fn start(devices: &[EndpointId]) -> Self {
        init_test_logging();
        let push = PushGateway::start().await;
        let matrix = TestServer::with_push_gateway(&push).await;
        let core = Arc::new(Core::new(
            matrix.url.trim_start_matches("http://").to_owned(),
            matrix.issuer.clone(),
        ));
        let (addr, task) = fake_core(core.clone()).await;
        let tunnel_key = SecretKey::generate();
        let endpoint = loopback_endpoint(tunnel_key.clone(), 0).await;
        let tunnel_port = endpoint.bound_sockets()[0].port();
        let allowlist = Allowlist::new(devices.iter().copied());
        let pairing = Arc::new(Pairing::new(allowlist.clone()));
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
}

pub use super::timeline::Timeline;
