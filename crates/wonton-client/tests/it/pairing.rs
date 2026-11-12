//! 扫码配对和添加设备的行为测试：只经导出给原生壳的接口驱动。服务端是只走回环地址的隧道入口，配对的处理方按发出去的
//! 码决定登记还是拒绝，登记就是把设备加进隧道的白名单；经隧道来的 HTTP 请求由一个模拟的核心回答。

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

const WAIT: Duration = Duration::from_secs(15);

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
            relay_urls: Vec::new(),
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
        let endpoint = Endpoint::builder(presets::Minimal)
            .secret_key(SecretKey::generate())
            .clear_ip_transports()
            .bind_addr((Ipv4Addr::LOCALHOST, 0))
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
            device_name: "Jiale 的 iPhone".to_owned(),
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

    async fn pairing_failure(&self) -> PairingFailure {
        let view = self
            .wait_for(|view| matches!(view.pairing, Some(PairingStatus::Failed { .. })))
            .await;
        match view.pairing {
            Some(PairingStatus::Failed { reason, .. }) => reason,
            _ => unreachable!(),
        }
    }
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
    assert_eq!(*home.codes.names.lock().unwrap(), vec!["Jiale 的 iPhone"]);
    drop(phone);

    // 存下的是服务端的地址，不是一次性的码：重启后直接连上，不用再配对。
    let restarted = Phone::open(dir.path());
    restarted
        .wait_for(|view| view.status == HomeStatus::Connected)
        .await;
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
        assert_eq!(phone.pairing_failure().await, failure, "{outcome:?}");
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
    assert_eq!(phone.pairing_failure().await, PairingFailure::Protocol);
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
    // 码里写的是本设备连这台服务器用的直连地址。
    let request: serde_json::Value =
        serde_json::from_str(&home.codes.invites.lock().unwrap()[0]).unwrap();
    let addrs: Vec<String> = home
        .codes
        .direct_addrs
        .iter()
        .map(ToString::to_string)
        .collect();
    assert_eq!(request["direct_addrs"], json!(addrs));

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
