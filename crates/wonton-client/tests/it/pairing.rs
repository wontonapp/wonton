//! 扫码配对和添加设备的行为测试：只经导出给原生壳的接口驱动。服务端是只走回环地址的隧道入口，配对的处理方按发出去的
//! 码决定登记还是拒绝，登记就是把设备加进隧道的白名单；经隧道来的 HTTP 请求由一个模拟的核心回答。

mod reset;
mod stuck_bind;

use std::collections::HashMap;
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
    PairingFailure, PairingStatus, SecretString,
};
use wonton_tunnel::pair::{
    PairHandler, PairProtocol, PairResponse, PairingCode, PairingSecret, RejectReason, SECRET_LEN,
};
use wonton_tunnel::{Allowlist, PeerStream, TunnelListener, TunnelServer};

/// 兜底预算，不是节奏：配对要另开 endpoint、建连再交换，繁忙时建连实测 16s+；取 60s，
/// 超过就是真的配不上。
const WAIT: Duration = Duration::from_secs(60);

/// 服务端发过的码：密钥对应配对时要回的结果，`None` 表示配对成功。
#[derive(Debug)]
struct Codes {
    allowlist: Allowlist,
    server_id: EndpointId,
    direct_addrs: Vec<SocketAddr>,
    codes: Mutex<HashMap<String, Option<RejectReason>>>,
    names: Mutex<Vec<String>>,
    /// 设备经隧道要码（添加设备）时的请求体。
    invites: Mutex<Vec<String>>,
    /// 要码时回这个 HTTP 状态码的错误，为空时发一个新码。
    refuse: Mutex<Option<u16>>,
    reported_addrs: Mutex<Vec<SocketAddr>>,
}

impl Codes {
    /// 发一个码，配对时按 `outcome` 回应。
    fn issue(&self, outcome: Option<RejectReason>) -> String {
        let mut codes = self.codes.lock().unwrap();
        let secret =
            PairingSecret::from_bytes([u8::try_from(codes.len() + 1).unwrap(); SECRET_LEN]);
        codes.insert(secret.expose().to_owned(), outcome);
        PairingCode {
            server_id: self.server_id,
            relays: Vec::new(),
            direct_addrs: self.direct_addrs.clone(),
            base_domain: "wonton.localhost".to_owned(),
            core_url: Url::from_str("http://wonton.localhost:18480/").unwrap(),
            secret,
        }
        .to_uri()
    }
}

impl PairHandler for Codes {
    fn pair(&self, device: EndpointId, secret: &PairingSecret, device_name: &str) -> PairResponse {
        match self.codes.lock().unwrap().get(secret.expose()) {
            Some(None) => {
                self.allowlist.allow(device);
                self.names.lock().unwrap().push(device_name.to_owned());
                PairResponse::Paired
            }
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

    async fn start_at(address: SocketAddr) -> Self {
        let endpoint = Endpoint::builder(presets::Minimal)
            .secret_key(SecretKey::generate())
            .clear_ip_transports()
            .bind_addr(address)
            .unwrap()
            .bind()
            .await
            .unwrap();
        let allowlist = Allowlist::default();
        let codes = Arc::new(Codes {
            allowlist: allowlist.clone(),
            server_id: endpoint.id(),
            direct_addrs: endpoint.bound_sockets(),
            codes: Mutex::new(HashMap::new()),
            names: Mutex::new(Vec::new()),
            invites: Mutex::new(Vec::new()),
            refuse: Mutex::new(None),
            reported_addrs: Mutex::new(Vec::new()),
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
    assert_eq!(phone.home().pairing, Some(PairingStatus::InProgress));
    // Existing pairing deadline is 30s. Two seconds only cover scheduling; QUIC cleanup must not delay the visible result.
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
    assert_eq!(phone.home().pairing, Some(PairingStatus::InProgress));
}

#[tokio::test]
async fn scanning_a_code_pairs_the_phone_and_it_reconnects_after_restart() {
    let home = Home::start().await;
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    phone.pair(&home.code(None)).unwrap();
    let view = phone
        .wait_for(|view| view.pairing.is_none() && view.status == HomeStatus::Connected)
        .await;
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
    phone
        .wait_for(|view| view.pairing.is_none() && view.status == HomeStatus::Connected)
        .await;
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
    phone
        .wait_for(|view| view.pairing.is_none() && view.status == HomeStatus::Connected)
        .await;
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
    phone
        .wait_for(|view| view.pairing.is_none() && view.status == HomeStatus::Connected)
        .await;

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
    tablet
        .wait_for(|view| view.pairing.is_none() && view.status == HomeStatus::Connected)
        .await;
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
