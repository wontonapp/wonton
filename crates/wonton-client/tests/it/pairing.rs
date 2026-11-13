//! 扫码配对和添加设备的行为测试：只经导出给原生壳的接口驱动。服务端是只走回环地址的隧道入口，配对的处理方按发出去的
//! 码决定登记还是拒绝，登记就是把设备加进隧道的白名单；经隧道来的 HTTP 请求由一个模拟的核心回答。

mod reset;
mod stuck_bind;

use std::collections::{HashMap, HashSet};
use std::net::{Ipv4Addr, SocketAddr};
use std::ops::Deref;
use std::str::FromStr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use iroh::endpoint::presets;
use iroh::{Endpoint, EndpointId, SecretKey};
use serde_json::json;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use url::Url;
use wonton_client::{
    Action, ClientCore, CoreError, DeviceInvite, HomeFailure, HomeStatus, HomeViewModel, Network,
    PairingFailure, PairingStage, PairingStatus, SecretString,
};
use wonton_protocol::{AdminScope, AiLevel, AppScope, Gate, JoinOutcome, Permissions, RoleDef};
use wonton_tunnel::pair::{
    PairHandler, PairProtocol, PairResponse, PairingCode, PairingSecret, RejectReason, SECRET_LEN,
};
use wonton_tunnel::{Allowlist, PeerStream, TunnelListener, TunnelServer};

/// 兜底预算，不是节奏：配对要另开 endpoint、建连再交换，繁忙时建连实测 16s+；连不上的服务端还要再确认一次
/// （两段 30 秒），取 90s，超过就是真的配不上。
const WAIT: Duration = Duration::from_secs(90);

/// 配上且连上了：核心把进展停在 `Paired`（壳离开配对页的类别），连接状态连着。
fn paired(view: &HomeViewModel) -> bool {
    matches!(view.pairing, Some(PairingStatus::Paired)) && view.status == HomeStatus::Connected
}

/// 服务端发过的码：密钥对应配对时要回的结果，`None` 表示配对成功。
#[derive(Debug)]
struct Codes {
    allowlist: Allowlist,
    server_id: EndpointId,
    direct_addrs: Vec<SocketAddr>,
    codes: Mutex<HashMap<String, Option<RejectReason>>>,
    /// 加入别人家的码：配对时回这个结果和角色，不登记设备。
    joins: Mutex<HashMap<String, (JoinOutcome, Option<RoleDef>)>>,
    names: Mutex<Vec<String>>,
    /// 设备经隧道要码（添加设备）时的请求体。
    invites: Mutex<Vec<String>>,
    /// 要码时回这个 HTTP 状态码的错误，为空时发一个新码。
    refuse: Mutex<Option<u16>>,
    reported_addrs: Mutex<Vec<SocketAddr>>,
    /// 接下来这么多个配对请求都拖住：每个都在处理里睡 [`SLOW_REPLY`] 才回，模拟慢中继上的回应。
    slow: Mutex<u32>,
    /// 已经见过的（码, 设备）。
    seen: Mutex<HashSet<(String, EndpointId)>>,
}

/// 比测试里的时限长得多：回应到之前就已经超时。
const SLOW_REPLY: Duration = Duration::from_secs(8);

impl Codes {
    /// 发一个码，配对时按 `outcome` 回应。
    fn issue(&self, outcome: Option<RejectReason>) -> String {
        self.issue_at(outcome, None)
    }

    /// 同上，直连地址换成 `addr`（拦在中间的回环转发）。
    fn issue_at(&self, outcome: Option<RejectReason>, addr: Option<SocketAddr>) -> String {
        let mut codes = self.codes.lock().unwrap();
        let secret =
            PairingSecret::from_bytes([u8::try_from(codes.len() + 1).unwrap(); SECRET_LEN]);
        codes.insert(secret.expose().to_owned(), outcome);
        PairingCode {
            server_id: self.server_id,
            relays: Vec::new(),
            direct_addrs: addr.map_or_else(|| self.direct_addrs.clone(), |addr| vec![addr]),
            base_domain: "wonton.localhost".to_owned(),
            core_url: Url::from_str("http://wonton.localhost:18480/").unwrap(),
            secret,
        }
        .to_uri()
    }
}

impl PairHandler for Codes {
    fn pair(&self, device: EndpointId, secret: &PairingSecret, device_name: &str) -> PairResponse {
        let key = (secret.expose().to_owned(), device);
        if let Some((outcome, role)) = self.joins.lock().unwrap().get(secret.expose()).cloned() {
            return PairResponse::Joining { outcome, role };
        }
        let first = self.seen.lock().unwrap().insert(key);
        // 登记先于回应，和真的核心一样：回应慢不代表登记慢，确认请求也不跟着一起慢。
        if first && matches!(self.codes.lock().unwrap().get(secret.expose()), Some(None)) {
            self.allowlist.allow(device);
            self.names.lock().unwrap().push(device_name.to_owned());
        }
        if *self.slow.lock().unwrap() > 0 {
            *self.slow.lock().unwrap() -= 1;
            std::thread::sleep(SLOW_REPLY);
        }
        match self.codes.lock().unwrap().get(secret.expose()) {
            Some(None) => PairResponse::Paired,
            Some(Some(reason)) => PairResponse::Rejected {
                reason: *reason,
                message: format!("{reason:?}"),
            },
            None => PairResponse::Rejected {
                reason: RejectReason::UnknownCode,
                message: "没发过".to_owned(),
            },
        }
    }
}

struct Home {
    server: TunnelServer,
    codes: Arc<Codes>,
}

impl Home {
    async fn start() -> Self {
        Self::start_at((Ipv4Addr::LOCALHOST, 0).into()).await
    }

    /// 起在给定地址上。同一端口重建时端口可能还被旧 endpoint 占着、或者刚空出来就被别的测试拿走，
    /// 所以反复 bind 到绑上为止，不先探再绑。
    async fn start_at(address: SocketAddr) -> Self {
        let key = SecretKey::generate();
        let deadline = tokio::time::Instant::now() + WAIT;
        let endpoint = loop {
            match Endpoint::builder(presets::Minimal)
                .secret_key(key.clone())
                .clear_ip_transports()
                .bind_addr(address)
                .unwrap()
                .bind()
                .await
            {
                Ok(endpoint) => break endpoint,
                Err(error) if crate::home::support::port_in_use(&error) => {
                    assert!(
                        tokio::time::Instant::now() < deadline,
                        "{address} 一直没有释放：{error}"
                    );
                    tokio::time::sleep(Duration::from_millis(1)).await;
                }
                Err(error) => panic!("{address} 起不来：{error}"),
            }
        };
        let allowlist = Allowlist::default();
        let codes = Arc::new(Codes {
            allowlist: allowlist.clone(),
            server_id: endpoint.id(),
            direct_addrs: endpoint.bound_sockets(),
            codes: Mutex::new(HashMap::new()),
            joins: Mutex::new(HashMap::new()),
            names: Mutex::new(Vec::new()),
            invites: Mutex::new(Vec::new()),
            refuse: Mutex::new(None),
            reported_addrs: Mutex::new(Vec::new()),
            slow: Mutex::new(0),
            seen: Mutex::new(HashSet::new()),
        });
        let (server, listener) = TunnelServer::spawn_embedded_with_pairing(
            endpoint,
            allowlist,
            PairProtocol::new(codes.clone()),
        );
        tokio::spawn(fake_core(listener, codes.clone()));
        Self { server, codes }
    }

    fn code(&self, outcome: Option<RejectReason>) -> String {
        self.codes.issue(outcome)
    }

    /// 直连地址换成 `addr` 的码。
    fn code_at(&self, addr: SocketAddr) -> String {
        self.codes.issue_at(None, Some(addr))
    }

    /// 发一个加入别人家的码：配对时回 `outcome`，等批时带 `role`。
    fn join_code(&self, outcome: JoinOutcome, role: Option<RoleDef>) -> String {
        let code = self.code(None);
        let secret = PairingCode::parse(&code)
            .unwrap()
            .secret
            .expose()
            .to_owned();
        self.codes
            .joins
            .lock()
            .unwrap()
            .insert(secret, (outcome, role));
        code
    }
}

/// 模拟的核心：经隧道来的 `POST /_wonton/api/pairing-codes`（添加设备）发一个新码，或者按 `Codes::refuse` 回错误。
async fn fake_core(mut listener: TunnelListener, codes: Arc<Codes>) {
    while let Some(stream) = listener.accept().await {
        tokio::spawn(answer(stream, codes.clone()));
    }
}

async fn answer(mut stream: PeerStream, codes: Arc<Codes>) {
    let mut head = Vec::new();
    while !head.ends_with(b"\r\n\r\n") {
        let mut byte = [0u8; 1];
        if stream.read(&mut byte).await.unwrap_or(0) == 0 {
            return;
        }
        head.push(byte[0]);
    }
    let text = String::from_utf8_lossy(&head).to_ascii_lowercase();
    let length = text
        .lines()
        .find_map(|line| line.strip_prefix("content-length: "))
        .map_or(0, |value| value.trim().parse().unwrap());
    let mut body = vec![0u8; length];
    stream.read_exact(&mut body).await.unwrap();
    let (status, reply) = if text.starts_with("post /_wonton/api/pairing-codes ") {
        codes
            .invites
            .lock()
            .unwrap()
            .push(String::from_utf8(body).unwrap());
        let refused = *codes.refuse.lock().unwrap();
        match refused {
            Some(status) => (
                status,
                json!({"error": {"kind": "refused", "message": "不给"}}),
            ),
            None => (200, json!({"code": codes.issue(None), "expires_in": 600})),
        }
    } else if text.starts_with("get /_wonton/api/tunnel ") {
        (
            200,
            json!({
                "endpoint_id": codes.server_id,
                "relays": [],
                "direct_addrs": *codes.reported_addrs.lock().unwrap(),
            }),
        )
    } else if text.starts_with("get /after-restart ") {
        (200, json!({"device": stream.remote_id().to_string()}))
    } else {
        (
            404,
            json!({"error": {"kind": "not_found", "message": "没有这个接口"}}),
        )
    };
    let reply = reply.to_string();
    let response = format!(
        "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\
         connection: close\r\n\r\n{reply}",
        reply.len()
    );
    stream.write_all(response.as_bytes()).await.ok();
    stream.shutdown().await.ok();
}

struct Phone {
    core: Arc<ClientCore>,
}

impl Deref for Phone {
    type Target = ClientCore;

    fn deref(&self) -> &ClientCore {
        &self.core
    }
}

impl Phone {
    fn open(dir: &std::path::Path) -> Self {
        let core = ClientCore::with_network(dir.to_str().unwrap(), Network::DirectOnly).unwrap();
        Self { core }
    }

    fn pair(&self, code: &str) -> Result<(), CoreError> {
        self.dispatch(Action::PairHome {
            code: SecretString::new(code),
            device_name: "Alice 的 iPhone".to_owned(),
        })
    }

    async fn wait_for(&self, mut done: impl FnMut(&HomeViewModel) -> bool) -> HomeViewModel {
        let subscription = self.subscribe_home();
        let waiting = async {
            while let Some(view) = subscription.next().await {
                if done(&view) {
                    return view;
                }
            }
            panic!("订阅提前结束");
        };
        tokio::time::timeout(WAIT, waiting)
            .await
            .unwrap_or_else(|_| panic!("等待超时，当前：{:?}", self.home()))
    }

    /// 失败的原因连同原文一起带出去：断言消息里要能看到 TunnelError 的原文，CI 再红时才能区分
    /// 服务端先关（LINGER）和服务端没读到请求（REQUEST_TIMEOUT）两个失败窗口。
    async fn pairing_failure(&self) -> (PairingFailure, String) {
        let view = self
            .wait_for(|view| matches!(view.pairing, Some(PairingStatus::Failed { .. })))
            .await;
        match view.pairing {
            Some(PairingStatus::Failed { reason, message }) => (reason, message),
            _ => unreachable!(),
        }
    }
}
/// 中继地址写成 http：码在本机就解析不过，当场报“要用 https”，不停在配对页等满时限。
#[tokio::test]
async fn a_code_with_an_http_relay_is_rejected_at_once() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let id = SecretKey::generate().public();
    let relays =
        serde_json::to_string(&json!([{ "url": "http://127.0.0.1:19194/", "tier": 0 }])).unwrap();
    let code = format!(
        "wonton://pair?v=0&id={id}&relays={}&addr=127.0.0.1:1&domain=wonton.localhost&core=http%3A%2F%2Fwonton.localhost%3A1%2F&secret={}",
        url::form_urlencoded::byte_serialize(relays.as_bytes()).collect::<String>(),
        "42".repeat(SECRET_LEN),
    );
    let error = phone.pair(&code).unwrap_err();
    let text = format!("{error:?}");
    assert!(text.contains("https"), "{text}");
}

#[tokio::test]
async fn unreachable_pairing_stops_before_cleanup_and_retry_starts_again() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let dead = std::net::UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let code = PairingCode {
        server_id: SecretKey::generate().public(),
        relays: Vec::new(),
        direct_addrs: vec![dead.local_addr().unwrap()],
        base_domain: "wonton.localhost".into(),
        core_url: Url::parse("http://wonton.localhost:18480/").unwrap(),
        secret: PairingSecret::from_bytes([42; SECRET_LEN]),
    }
    .to_uri();
    phone.pair(&code).unwrap();
    assert!(matches!(
        phone.home().pairing,
        Some(PairingStatus::InProgress { .. })
    ));
    // 建连那一段 30 秒，超时就报连不上（不再确认）。几秒只给调度和 QUIC 清理留余量：
    // 清理不能推迟可见的结果。
    let (failure, message) = tokio::time::timeout(Duration::from_secs(32), phone.pairing_failure())
        .await
        .expect("pairing failure must be published before endpoint cleanup");
    assert_eq!(failure, PairingFailure::TransportUnavailable, "{message}");
    assert_eq!(phone.home().status, HomeStatus::NotConfigured);
    assert!(phone.home().server_id.is_none());
    phone.dispatch(Action::DismissPairingFailure).unwrap();
    assert!(phone.home().pairing.is_none());
    assert_eq!(phone.home().status, HomeStatus::NotConfigured);
    phone.pair(&code).unwrap();
    assert!(matches!(
        phone.home().pairing,
        Some(PairingStatus::InProgress { .. })
    ));
}

/// 连不上的地址：配对码交不出去，进度一直停在“正在连接”，已等待的时间由核心按秒推进——壳的“还在连接”
/// 和取消按钮就从这个状态来。取消后回到没有配对的状态；没有配对时取消是输入错误。
#[tokio::test]
async fn an_unreachable_server_stays_connecting_and_the_wait_grows() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let dead = std::net::UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let code = PairingCode {
        server_id: SecretKey::generate().public(),
        relays: Vec::new(),
        direct_addrs: vec![dead.local_addr().unwrap()],
        base_domain: "wonton.localhost".into(),
        core_url: Url::parse("http://wonton.localhost:18480/").unwrap(),
        secret: PairingSecret::from_bytes([42; SECRET_LEN]),
    }
    .to_uri();
    phone.pair(&code).unwrap();
    // 5 秒是壳补“还在连接”的门槛：到那时核心已经给出了真实的等待时间和步骤。
    phone
        .wait_for(|view| {
            matches!(
                view.pairing,
                Some(PairingStatus::InProgress {
                    elapsed_ms: 5_000..,
                    stage: PairingStage::Connecting
                })
            )
        })
        .await;
    phone.dispatch(Action::CancelPairing).unwrap();
    assert!(phone.home().pairing.is_none());
    assert_eq!(phone.home().status, HomeStatus::NotConfigured);
    assert!(
        matches!(
            phone.dispatch(Action::CancelPairing),
            Err(CoreError::InvalidInput { .. })
        ),
        "没有正在进行的配对时取消该报输入错误"
    );
}

/// 慢回应：配对码交出后进度翻到“等服务器确认”；取消回到可以重新输入的状态，服务端迟到的登记不算数，
/// 同一个码再配一次，服务端对已登记的设备幂等地回成功。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancelling_a_slow_pairing_returns_to_the_form_and_a_retry_pairs() {
    let home_side = Home::start().await;
    *home_side.codes.slow.lock().unwrap() = 1;
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let code = home_side.code(None);
    phone.pair(&code).unwrap();
    // 直连建连很快，码交出就翻步；时间由心跳按秒推进，第一次翻步时至少过了一秒。
    phone
        .wait_for(|view| {
            matches!(
                view.pairing,
                Some(PairingStatus::InProgress {
                    elapsed_ms: 1_000..,
                    stage: PairingStage::Confirming
                })
            )
        })
        .await;
    phone.dispatch(Action::CancelPairing).unwrap();
    assert!(phone.home().pairing.is_none());
    assert_eq!(phone.home().status, HomeStatus::NotConfigured);
    // 取消不是配上：核心不给 Paired，壳的“配上了”只看这个类别，取消回到没有进展。
    assert!(phone.home().server_id.is_none());
    // 迟到的回应不算数：取消之后状态不再变。
    tokio::time::sleep(SLOW_REPLY).await;
    assert!(phone.home().pairing.is_none(), "{:?}", phone.home());
    phone.pair(&code).unwrap();
    let view = phone.wait_for(paired).await;
    assert_eq!(
        view.server_id,
        Some(home_side.server.endpoint().id().to_string())
    );
}

#[tokio::test]
async fn scanning_a_code_pairs_the_phone_and_it_reconnects_after_restart() {
    let home = Home::start().await;
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    phone.pair(&home.code(None)).unwrap();
    let view = phone.wait_for(paired).await;
    assert_eq!(
        view.server_id,
        Some(home.server.endpoint().id().to_string())
    );
    assert!(view.proxy.is_some());
    assert_eq!(*home.codes.names.lock().unwrap(), vec!["Alice 的 iPhone"]);
    drop(phone);

    // 存下的是服务端的地址，不是一次性的码：重启后直接连上，不用再配对。
    let restarted = Phone::open(dir.path());
    restarted
        .wait_for(|view| view.status == HomeStatus::Connected)
        .await;
}

#[tokio::test]
async fn reported_addresses_do_not_erase_the_working_route_after_pairing() {
    reconnects_after_report(1).await;
}

#[tokio::test]
async fn a_full_report_cannot_evict_the_working_route_after_pairing() {
    reconnects_after_report(8).await;
}

async fn reconnects_after_report(count: usize) {
    let home = Home::start().await;
    let unreachable: Vec<_> = (0..count)
        .map(|_| std::net::UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap())
        .collect();
    let reported: Vec<_> = unreachable
        .iter()
        .map(|socket| socket.local_addr().unwrap())
        .collect();
    let mut code = PairingCode::parse(&home.code(None)).unwrap();
    let connected = code.direct_addrs[0];
    let stale: SocketAddr = "192.0.2.10:18482".parse().unwrap();
    code.direct_addrs.push(stale);
    *home.codes.reported_addrs.lock().unwrap() = reported.clone();
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    phone.pair(&code.to_uri()).unwrap();
    phone.wait_for(paired).await;
    let ticket_path = dir.path().join("servers/1/home-ticket.json");
    tokio::time::timeout(WAIT, async {
        loop {
            let saved = std::fs::read_to_string(&ticket_path).unwrap();
            if saved.contains(&reported[0].to_string()) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("核心报来的地址没有存进连接信息");
    let device = phone.home().device_id;
    let saved: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&ticket_path).unwrap()).unwrap();
    drop(phone);

    let restarted = Phone::open(dir.path());
    let view = restarted
        .wait_for(|view| view.status == HomeStatus::Connected && view.proxy.is_some())
        .await;
    let proxy = view.proxy.unwrap();
    let response = reqwest::Client::builder()
        .proxy(
            reqwest::Proxy::all(format!("http://{}:{}", proxy.host, proxy.port))
                .unwrap()
                .basic_auth(&proxy.username, &proxy.password),
        )
        .timeout(WAIT)
        .build()
        .unwrap()
        .get("http://wonton.localhost:18480/after-restart")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&response.bytes().await.unwrap()).unwrap(),
        json!({"device": device})
    );
    assert!(saved["server"]["relays"].as_array().unwrap().is_empty());
    assert!(
        !saved["server"]["direct_addrs"]
            .as_array()
            .unwrap()
            .contains(&json!(stale))
    );
    let expected: Vec<_> = std::iter::once(connected)
        .chain(reported.iter().copied())
        .take(8)
        .collect();
    assert_eq!(saved["server"]["direct_addrs"], json!(expected));
}

#[tokio::test]
async fn a_rejected_code_says_what_to_do_next() {
    let home = Home::start().await;
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    for (outcome, failure) in [
        (Some(RejectReason::Expired), PairingFailure::Expired),
        (Some(RejectReason::Used), PairingFailure::Rejected),
        (Some(RejectReason::Closed), PairingFailure::Closed),
        (Some(RejectReason::ServerError), PairingFailure::Upstream),
    ] {
        phone.pair(&home.code(outcome)).unwrap();
        let (got, message) = phone.pairing_failure().await;
        assert_eq!(got, failure, "{outcome:?}: {message}");
        assert_eq!(
            phone.home().status,
            HomeStatus::NotConfigured,
            "配不上就不存连接信息"
        );
    }
    // 拒绝之后换一个好码照样能配上。
    phone.pair(&home.code(None)).unwrap();
    phone.wait_for(paired).await;
}

/// 加入别人家的码：等批如实报出角色、码被用过说要换码，管理员同意后用同一个码重试能配上。
#[tokio::test]
async fn a_joining_code_waits_for_approval_and_the_same_code_pairs_once_approved() {
    let home = Home::start().await;
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let role = RoleDef {
        id: "child".to_owned(),
        name: "孩子".to_owned(),
        template: None,
        permissions: Permissions {
            open_apps: AppScope::Other,
            plugin_admin: AdminScope::NoApps,
            install_apps: Gate::AskAdmin,
            model_settings: false,
            server_settings: false,
            manage_members: false,
            add_friends: Gate::Deny,
            share_apps: false,
            ai: AiLevel::ReadOnly,
            privileged_tools: false,
            private_bot: true,
            shared_bots: false,
            lend_own_bot: false,
        },
        people: 0,
    };
    let code = home.join_code(JoinOutcome::Pending, Some(role.clone()));
    phone.pair(&code).unwrap();
    let (failure, message) = phone.pairing_failure().await;
    assert_eq!(failure, PairingFailure::PendingApproval, "{message}");
    assert!(message.contains("孩子"), "{message}");
    assert_eq!(
        phone.home().status,
        HomeStatus::NotConfigured,
        "等批时还不存连接信息"
    );

    // 同一台设备再交同一个码还是等批；换成被别的设备用过的码就报换码。
    phone.pair(&code).unwrap();
    let (failure, message) = phone.pairing_failure().await;
    assert_eq!(failure, PairingFailure::PendingApproval, "{message}");
    let used = home.join_code(JoinOutcome::CodeUsed, None);
    phone.pair(&used).unwrap();
    let (failure, message) = phone.pairing_failure().await;
    assert_eq!(failure, PairingFailure::Rejected, "{message}");

    // 管理员同意了：同一个码再交，服务端直接登记设备、回 Paired。
    let secret = PairingCode::parse(&code)
        .unwrap()
        .secret
        .expose()
        .to_owned();
    home.codes.joins.lock().unwrap().remove(&secret);
    home.codes.codes.lock().unwrap().insert(secret, None);
    phone.pair(&code).unwrap();
    phone.wait_for(paired).await;
}

#[tokio::test]
async fn other_qr_codes_are_rejected_and_newer_codes_ask_for_an_update() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let before = phone.home();
    for text in ["", "https://example.com/", "wonton://pair?v=0"] {
        assert!(
            matches!(phone.pair(text), Err(CoreError::InvalidInput { .. })),
            "{text}"
        );
    }
    assert_eq!(phone.home(), before);

    let home = Home::start().await;
    let newer = home.code(None).replace("v=0", "v=9");
    phone.pair(&newer).unwrap();
    let (failure, message) = phone.pairing_failure().await;
    assert_eq!(failure, PairingFailure::Protocol, "{message}");
}

#[tokio::test]
async fn a_paired_phone_gets_a_code_that_adds_another_device() {
    let home = Home::start().await;
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    assert!(
        matches!(
            phone.dispatch(Action::InviteDevice),
            Err(CoreError::NotReady { .. })
        ),
        "还没配对时不能添加设备"
    );
    phone.pair(&home.code(None)).unwrap();
    phone.wait_for(paired).await;

    phone.dispatch(Action::InviteDevice).unwrap();
    let view = phone
        .wait_for(|view| matches!(view.invite, Some(DeviceInvite::Ready { .. })))
        .await;
    let Some(DeviceInvite::Ready {
        code,
        expires_at_ms,
    }) = view.invite
    else {
        unreachable!()
    };
    let now_ms = u64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis(),
    )
    .unwrap();
    assert!(expires_at_ms > now_ms + 590_000, "码 10 分钟后过期");
    // 直连地址由核心算，本设备不再把自己的连接信息传过去：请求体是空的。
    let request: serde_json::Value =
        serde_json::from_str(&home.codes.invites.lock().unwrap()[0]).unwrap();
    assert_eq!(request, json!({}));
    assert!(
        home.codes.direct_addrs[0].port() != 0,
        "假核心的码里仍有地址"
    );

    // 新设备扫这个码配上。
    let other = tempfile::tempdir().unwrap();
    let tablet = Phone::open(other.path());
    tablet.pair(code.expose()).unwrap();
    tablet.wait_for(paired).await;
}

#[tokio::test]
async fn a_refused_invite_says_why() {
    let home = Home::start().await;
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    phone.pair(&home.code(None)).unwrap();
    phone
        .wait_for(|view| view.status == HomeStatus::Connected)
        .await;
    for (status, failure) in [
        (403, HomeFailure::Unauthorized),
        (503, HomeFailure::UpstreamUnavailable),
        (404, HomeFailure::Protocol),
    ] {
        *home.codes.refuse.lock().unwrap() = Some(status);
        phone.dispatch(Action::InviteDevice).unwrap();
        let view = phone
            .wait_for(|view| matches!(view.invite, Some(DeviceInvite::Failed { .. })))
            .await;
        match view.invite {
            Some(DeviceInvite::Failed { reason, .. }) => assert_eq!(reason, failure, "{status}"),
            _ => unreachable!(),
        }
    }
}

/// 测试里的每一段时限：留够冷启动的绑定和建连，慢回应还是它的两倍多。多线程运行时是必需的：假服务端
/// 用阻塞睡眠拖住回应，单线程运行时里客户端自己的计时器也会跟着停住。
const STAGE: Duration = Duration::from_secs(3);
const WATCH: Duration = Duration::from_secs(30);

/// 回应在超时之后才到（中继慢）：请求已经送到、服务端也登记了，客户端不能报失败——确认一次就配上。
/// 确认和第一次都赶不上才报失败，那时码已经用掉，回 `used`。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_reply_after_the_deadline_still_pairs() {
    let home = Home::start().await;
    *home.codes.slow.lock().unwrap() = 1;
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    phone.test_set_pair_deadlines(STAGE, WATCH);
    phone.pair(&home.code(None)).unwrap();
    let view = phone.wait_for(paired).await;
    assert_eq!(
        view.server_id,
        Some(home.server.endpoint().id().to_string())
    );
    assert_eq!(*home.codes.names.lock().unwrap(), vec!["Alice 的 iPhone"]);
    // 存的是服务端地址，不是码：重启后直接连上。
    drop(phone);
    let restarted = Phone::open(dir.path());
    restarted
        .wait_for(|view| view.status == HomeStatus::Connected)
        .await;
}

/// 建连那一段超时就不确认了：没有 0-RTT，握手没完成时请求一个字节都发不出去，服务端不可能登记，
/// 再问一次只会让死服务端多等一段。用一条拖慢第一个包的通道把握手拖过时限，断言只发生了一次交换。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_connect_that_misses_the_stage_fails_without_a_second_exchange() {
    let home = Home::start().await;
    // 设备→服务端的包都晚 1.5 秒才转：握手赶不上 1 秒的时限，整段交换超时。
    let proxy = crate::home::support::SlowUdp::start_first(
        home.server.endpoint().bound_sockets()[0],
        1000,
        Duration::from_millis(1500),
    )
    .await;
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    phone.test_set_pair_deadlines(Duration::from_secs(1), WATCH);
    phone.pair(&home.code_at(proxy.addr())).unwrap();
    let (failure, message) = phone.pairing_failure().await;
    assert_eq!(failure, PairingFailure::TransportUnavailable, "{message}");
    assert_eq!(phone.home().status, HomeStatus::NotConfigured);
    let device: EndpointId = phone.home().device_id.parse().unwrap();
    assert!(
        !home.codes.allowlist.contains(&device),
        "握手都没完成，服务端不该登记"
    );
}

/// 第一次和自动的确认都超过时限才报失败；服务端已经登记了这台设备，拿同一个码手动重试还能配上，
/// 不会停在“码已经用过”的死胡同里。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn retrying_the_same_code_after_a_failure_pairs() {
    let home = Home::start().await;
    *home.codes.slow.lock().unwrap() = 2;
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    phone.test_set_pair_deadlines(STAGE, WATCH);
    let code = home.code(None);
    phone.pair(&code).unwrap();
    let (failure, message) = phone.pairing_failure().await;
    assert_eq!(failure, PairingFailure::TransportUnavailable, "{message}");
    assert_eq!(phone.home().status, HomeStatus::NotConfigured);
    let device: EndpointId = phone.home().device_id.parse().unwrap();
    assert!(
        home.codes.allowlist.contains(&device),
        "服务端已经登记了这台设备"
    );

    // 手动重试本身也可能是冷启动：给它宽一点的一段，码已经不再慢，配上就完事。
    phone.test_set_pair_deadlines(Duration::from_secs(10), WATCH);
    phone.pair(&code).unwrap();
    phone.wait_for(paired).await;
}
