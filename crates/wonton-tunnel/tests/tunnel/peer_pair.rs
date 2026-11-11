//! 服务器配对的行为测试：两台“服务器”都在本机进程内，各自一个 endpoint 和服务器配对协议；
//! 配对业务（令牌、请求表、回答）用测试里的替身处理方模拟。不依赖外网和 Docker。
//!
//! 码里只有中继，测试的 endpoint 不开中继，所以都用 `scan_to` 直接连对方的套接字；码本身的形状单独测。

use std::{
    net::SocketAddr,
    sync::{Arc, Mutex},
};

use iroh::protocol::DynProtocolHandler;
use iroh::{EndpointId, SecretKey};
use wonton_tunnel::TunnelServer;
use wonton_tunnel::pair::{PairHandler, PairProtocol, PairResponse, RejectReason};
use wonton_tunnel::peer_pair::{
    PEER_PAIR_ALPN, PeerIdentity, PeerPairHandler, PeerPairProtocol, PeerPairToken,
    PeerPairingCode, PeerRejectReason, PeerReply, ask_outcome, deliver_decision, notify_removed,
    scan_to,
};

use crate::support::{Timeouts, loopback_endpoint};

/// 出码方的令牌和请求表。码长期有效，同一台服务器同时只有一条请求。
#[derive(Debug)]
struct Issuer {
    server_name: String,
    token: PeerPairToken,
    /// 已配对的（endpoint ID，服务器名）。
    paired: Mutex<Vec<(EndpointId, String)>>,
    /// 扫进来的请求：扫码方的 endpoint ID、身份和能被回连的地址。
    scanned: Mutex<Vec<(EndpointId, PeerIdentity)>>,
    /// 主人拒绝过的 endpoint ID。
    rejected: Mutex<Vec<EndpointId>>,
    /// 主人的回答：还没回答时为空。
    answer: Mutex<Option<bool>>,
}

impl Issuer {
    fn new() -> Self {
        Self {
            server_name: "issuer.example".to_owned(),
            token: PeerPairToken::from_bytes([7; 16]),
            paired: Mutex::new(Vec::new()),
            scanned: Mutex::new(Vec::new()),
            rejected: Mutex::new(Vec::new()),
            answer: Mutex::new(None),
        }
    }

    fn own_identity(&self) -> PeerIdentity {
        PeerIdentity {
            server_name: self.server_name.clone(),
            owner_user_id: "@alex:issuer.example".to_owned(),
            owner_display_name: "Alex".to_owned(),
            relays: Vec::new(),
            addrs: Vec::new(),
        }
    }
}

impl PeerPairHandler for Issuer {
    fn scanned(
        &self,
        scanner: EndpointId,
        identity: &PeerIdentity,
        token: &PeerPairToken,
    ) -> PeerReply {
        if identity.server_name == self.server_name {
            return PeerReply::Error {
                reason: PeerRejectReason::SelfPairing,
                message: "扫了自己的服务器".to_owned(),
            };
        }
        if *token != self.token {
            return PeerReply::Error {
                reason: PeerRejectReason::CodeRevoked,
                message: "这张码已经作废".to_owned(),
            };
        }
        if self.rejected.lock().unwrap().contains(&scanner) {
            return PeerReply::Error {
                reason: PeerRejectReason::Rejected,
                message: "主人拒绝过这台服务器".to_owned(),
            };
        }
        if self
            .paired
            .lock()
            .unwrap()
            .iter()
            .any(|(id, name)| *id == scanner || name == &identity.server_name)
        {
            return PeerReply::Error {
                reason: PeerRejectReason::AlreadyPaired,
                message: "已经配对过".to_owned(),
            };
        }
        // 同一台服务器再发只更新那一条。
        let mut scanned = self.scanned.lock().unwrap();
        match scanned.iter_mut().find(|(id, _)| *id == scanner) {
            Some(entry) => entry.1 = identity.clone(),
            None => scanned.push((scanner, identity.clone())),
        }
        PeerReply::AwaitingApproval {
            identity: self.own_identity(),
        }
    }

    fn outcome(&self, scanner: EndpointId) -> PeerReply {
        match *self.answer.lock().unwrap() {
            Some(true) => PeerReply::Approved {
                identity: self.own_identity(),
            },
            Some(false) => PeerReply::Declined,
            None if self
                .scanned
                .lock()
                .unwrap()
                .iter()
                .any(|(id, _)| *id == scanner) =>
            {
                PeerReply::Pending
            }
            None => PeerReply::UnknownRequest,
        }
    }

    fn expecting_decision(&self, _: &EndpointId) -> bool {
        false
    }

    fn decision(&self, _: EndpointId, _: bool, _: &PeerIdentity) {}

    fn removed(&self, _: EndpointId, _: &str) {}
}

impl PairHandler for Issuer {
    fn pair(&self, _: EndpointId, _: &wonton_tunnel::pair::PairingSecret, _: &str) -> PairResponse {
        PairResponse::Rejected {
            reason: RejectReason::Closed,
            message: "测试不开设备配对".into(),
        }
    }
}

/// 扫码方的状态：在等谁的结果、收到的结果和删除通知。
#[derive(Debug, Default)]
struct Scanner {
    /// 我们扫过的出码方 endpoint ID（码里带来的）。
    expecting: Mutex<Vec<EndpointId>>,
    decisions: Mutex<Vec<(EndpointId, bool, PeerIdentity)>>,
    removed: Mutex<Vec<(EndpointId, String)>>,
}

impl PeerPairHandler for Scanner {
    fn scanned(&self, _: EndpointId, _: &PeerIdentity, _: &PeerPairToken) -> PeerReply {
        PeerReply::Error {
            reason: PeerRejectReason::BadRequest,
            message: "扫码方不该收到扫码".to_owned(),
        }
    }

    fn outcome(&self, _: EndpointId) -> PeerReply {
        PeerReply::UnknownRequest
    }

    fn expecting_decision(&self, from: &EndpointId) -> bool {
        self.expecting.lock().unwrap().contains(from)
    }

    fn decision(&self, from: EndpointId, approved: bool, identity: &PeerIdentity) {
        self.decisions
            .lock()
            .unwrap()
            .push((from, approved, identity.clone()));
    }

    fn removed(&self, from: EndpointId, server_name: &str) {
        self.removed
            .lock()
            .unwrap()
            .push((from, server_name.to_owned()));
    }
}

impl PairHandler for Scanner {
    fn pair(&self, _: EndpointId, _: &wonton_tunnel::pair::PairingSecret, _: &str) -> PairResponse {
        PairResponse::Rejected {
            reason: RejectReason::Closed,
            message: "测试不开设备配对".into(),
        }
    }
}

struct Server {
    id: EndpointId,
    addrs: Vec<SocketAddr>,
    #[allow(dead_code)]
    server: TunnelServer,
}

impl Server {
    fn endpoint(&self) -> &iroh::Endpoint {
        self.server.endpoint()
    }
}

async fn start<H: PeerPairHandler + wonton_tunnel::pair::PairHandler>(handler: Arc<H>) -> Server {
    let endpoint = loopback_endpoint(SecretKey::generate(), 0, Timeouts::Default).await;
    let addrs = endpoint.bound_sockets();
    let (server, _listener) = TunnelServer::spawn_embedded_with_pairing_and(
        endpoint,
        wonton_tunnel::Allowlist::default(),
        PairProtocol::new(handler.clone()),
        [(
            PEER_PAIR_ALPN,
            Box::new(PeerPairProtocol::new(handler)) as Box<dyn DynProtocolHandler>,
        )],
    );
    Server {
        id: server.endpoint().id(),
        addrs,
        server,
    }
}

fn code_for(issuer: &Server, token: PeerPairToken) -> PeerPairingCode {
    PeerPairingCode {
        server_id: issuer.id,
        relay_urls: vec!["https://relay.example.com./".parse().unwrap()],
        server_name: "issuer.example".to_owned(),
        token,
    }
}

fn scanner_identity() -> PeerIdentity {
    PeerIdentity {
        server_name: "scanner.example".to_owned(),
        owner_user_id: "@alice:scanner.example".to_owned(),
        owner_display_name: "Alice".to_owned(),
        relays: Vec::new(),
        addrs: Vec::new(),
    }
}

/// 扫码方连出码方的地址：测试里没有中继，直连地址就是 endpoint 的套接字。
fn addr_for(id: EndpointId, addrs: &[SocketAddr]) -> iroh::EndpointAddr {
    let mut addr = iroh::EndpointAddr::new(id);
    for ip in addrs {
        addr = addr.with_ip_addr(*ip);
    }
    addr
}

/// 扫对方的码：连对方的套接字，出示令牌和自己的身份。码里没有直连地址时用它。
async fn scan_server(
    scanner: &Server,
    issuer: &Server,
    token: PeerPairToken,
    identity: PeerIdentity,
) -> Result<PeerReply, wonton_tunnel::TunnelError> {
    scan_to(
        scanner.endpoint(),
        addr_for(issuer.id, &issuer.addrs),
        token,
        identity,
    )
    .await
}

#[tokio::test]
async fn a_scan_gets_awaiting_approval_and_the_decision_reaches_only_the_scanned_endpoint() {
    let issuer_state = Arc::new(Issuer::new());
    let issuer = start(issuer_state.clone()).await;
    let scanner_state = Arc::new(Scanner::default());
    let scanner = start(scanner_state.clone()).await;

    // 扫码：出码方回了“等主人同意”，附自己的身份。
    scanner_state.expecting.lock().unwrap().push(issuer.id);
    let reply = scan_server(
        &scanner,
        &issuer,
        issuer_state.token.clone(),
        PeerIdentity {
            server_name: "scanner.example".to_owned(),
            owner_user_id: "@alice:scanner.example".to_owned(),
            owner_display_name: "Alice".to_owned(),
            relays: Vec::new(),
            addrs: scanner.addrs.clone(),
        },
    )
    .await
    .unwrap();
    let PeerReply::AwaitingApproval { identity } = reply else {
        panic!("应是等待同意，实际 {reply:?}");
    };
    assert_eq!(identity.server_name, "issuer.example");
    assert_eq!(identity.owner_display_name, "Alex");
    // 出码方记下了扫码方的身份和回连地址。
    let scanner_addr = {
        let scanned = issuer_state.scanned.lock().unwrap();
        let (scanner_id, scanner_ident) = &scanned[0];
        assert_eq!(*scanner_id, scanner.id, "扫码方身份来自 QUIC 握手");
        assert_eq!(scanner_ident.server_name, "scanner.example");
        addr_for(*scanner_id, &scanner_ident.addrs)
    };

    // 主人同意：结果送回扫码方。
    deliver_decision(
        issuer.endpoint(),
        scanner_addr.clone(),
        true,
        issuer_state.own_identity(),
    )
    .await
    .unwrap();
    {
        let decisions = scanner_state.decisions.lock().unwrap();
        assert_eq!(decisions.len(), 1);
        assert!(decisions[0].1, "应是同意");
        assert_eq!(decisions[0].2.server_name, "issuer.example");
    }

    // 别的 endpoint 冒充出码方送结果：扫码方不理，对方收到错误。
    let impostor = loopback_endpoint(SecretKey::generate(), 0, Timeouts::Default).await;
    let error = deliver_decision(
        &impostor,
        scanner_addr,
        true,
        PeerIdentity {
            server_name: "impostor.example".to_owned(),
            owner_user_id: "@evil:impostor.example".to_owned(),
            owner_display_name: "Evil".to_owned(),
            relays: Vec::new(),
            addrs: Vec::new(),
        },
    )
    .await
    .unwrap_err();
    assert!(
        matches!(
            error,
            wonton_tunnel::TunnelError::TransportUnavailable { .. }
        ),
        "{error:?}"
    );
    assert_eq!(scanner_state.decisions.lock().unwrap().len(), 1);
    issuer.server.shutdown().await;
    scanner.server.shutdown().await;
}

/// 码的契约：版本升到 1，只有 endpoint ID、中继、服务器名和令牌，没有直连地址。
#[tokio::test]
async fn the_code_speaks_version_one_and_carries_no_direct_address() {
    let issuer_state = Arc::new(Issuer::new());
    let issuer = start(issuer_state.clone()).await;
    let code = code_for(&issuer, issuer_state.token.clone());
    let uri = code.to_uri();
    assert!(uri.starts_with("wonton://peer?v=1&"), "{uri}");
    assert!(uri.contains("token="), "{uri}");
    assert!(!uri.contains("addr="), "{uri}");
    assert_eq!(PeerPairingCode::parse(&uri).unwrap(), code);
    assert_eq!(
        PeerPairingCode::parse(&uri.replace("v=1", "v=0")),
        Err(wonton_tunnel::peer_pair::InvalidPeerCode::UnsupportedVersion("0".to_owned()))
    );
    issuer.server.shutdown().await;
}

/// 同一张码扫两次都收下，第二次只更新那一条请求，不新开一条。
#[tokio::test]
async fn the_same_code_scans_twice_and_updates_one_request() {
    let issuer_state = Arc::new(Issuer::new());
    let issuer = start(issuer_state.clone()).await;
    let scanner = start(Arc::new(Scanner::default())).await;

    for name in ["Alice", "改名了"] {
        let reply = scan_server(
            &scanner,
            &issuer,
            issuer_state.token.clone(),
            PeerIdentity {
                owner_display_name: name.to_owned(),
                addrs: scanner.addrs.clone(),
                ..scanner_identity()
            },
        )
        .await
        .unwrap();
        assert!(
            matches!(reply, PeerReply::AwaitingApproval { .. }),
            "{reply:?}"
        );
    }
    {
        let scanned = issuer_state.scanned.lock().unwrap();
        assert_eq!(scanned.len(), 1, "同一台服务器只有一条请求");
        assert_eq!(scanned[0].1.owner_display_name, "改名了");
    }
    issuer.server.shutdown().await;
    scanner.server.shutdown().await;
}

/// 令牌不对（对方拿的是重置前的码）：按 `code_revoked` 拒绝；拒绝过的服务器再扫直接说清。
#[tokio::test]
async fn a_wrong_token_and_a_rejected_scanner_are_told_apart() {
    let issuer_state = Arc::new(Issuer::new());
    let issuer = start(issuer_state.clone()).await;
    let scanner = start(Arc::new(Scanner::default())).await;

    let reply = scan_server(
        &scanner,
        &issuer,
        PeerPairToken::from_bytes([9; 16]),
        scanner_identity(),
    )
    .await
    .unwrap();
    assert!(matches!(
        reply,
        PeerReply::Error {
            reason: PeerRejectReason::CodeRevoked,
            ..
        }
    ));

    issuer_state.rejected.lock().unwrap().push(scanner.id);
    let reply = scan_server(
        &scanner,
        &issuer,
        issuer_state.token.clone(),
        scanner_identity(),
    )
    .await
    .unwrap();
    assert!(matches!(
        reply,
        PeerReply::Error {
            reason: PeerRejectReason::Rejected,
            ..
        }
    ));
    assert!(issuer_state.scanned.lock().unwrap().is_empty());

    // 已经配对过。
    issuer_state.rejected.lock().unwrap().clear();
    issuer_state
        .paired
        .lock()
        .unwrap()
        .push((SecretKey::generate().public(), "scanner.example".to_owned()));
    let reply = scan_server(
        &scanner,
        &issuer,
        issuer_state.token.clone(),
        scanner_identity(),
    )
    .await
    .unwrap();
    assert!(matches!(
        reply,
        PeerReply::Error {
            reason: PeerRejectReason::AlreadyPaired,
            ..
        }
    ));
    issuer.server.shutdown().await;
    scanner.server.shutdown().await;
}

/// 扫码方来问结果：等待中、已同意（附出码方的身份）、已拒绝、不认识这条请求。
#[tokio::test]
async fn asking_for_the_outcome_reports_pending_approved_declined_and_unknown() {
    let issuer_state = Arc::new(Issuer::new());
    let issuer = start(issuer_state.clone()).await;
    let scanner = start(Arc::new(Scanner::default())).await;
    let stranger = loopback_endpoint(SecretKey::generate(), 0, Timeouts::Default).await;

    // 还没扫过：不认识。
    let reply = ask_outcome(scanner.endpoint(), addr_for(issuer.id, &issuer.addrs))
        .await
        .unwrap();
    assert_eq!(reply, PeerReply::UnknownRequest);

    scan_server(
        &scanner,
        &issuer,
        issuer_state.token.clone(),
        PeerIdentity {
            addrs: scanner.addrs.clone(),
            ..scanner_identity()
        },
    )
    .await
    .unwrap();
    let reply = ask_outcome(scanner.endpoint(), addr_for(issuer.id, &issuer.addrs))
        .await
        .unwrap();
    assert_eq!(reply, PeerReply::Pending);

    *issuer_state.answer.lock().unwrap() = Some(true);
    let reply = ask_outcome(scanner.endpoint(), addr_for(issuer.id, &issuer.addrs))
        .await
        .unwrap();
    let PeerReply::Approved { identity } = reply else {
        panic!("应是已同意，实际 {reply:?}");
    };
    assert_eq!(identity.server_name, "issuer.example");

    *issuer_state.answer.lock().unwrap() = Some(false);
    let reply = ask_outcome(&stranger, addr_for(issuer.id, &issuer.addrs))
        .await
        .unwrap();
    assert_eq!(reply, PeerReply::Declined);

    // 别的 endpoint 来问、主人又还没回答：它没发过请求。
    *issuer_state.answer.lock().unwrap() = None;
    let reply = ask_outcome(&stranger, addr_for(issuer.id, &issuer.addrs))
        .await
        .unwrap();
    assert_eq!(reply, PeerReply::UnknownRequest);
    issuer.server.shutdown().await;
    scanner.server.shutdown().await;
}

#[tokio::test]
async fn a_rejected_decision_and_a_removal_notice_reach_the_scanner() {
    let issuer_state = Arc::new(Issuer::new());
    let issuer = start(issuer_state.clone()).await;
    let scanner_state = Arc::new(Scanner::default());
    let scanner = start(scanner_state.clone()).await;

    scanner_state.expecting.lock().unwrap().push(issuer.id);
    scan_server(
        &scanner,
        &issuer,
        issuer_state.token.clone(),
        PeerIdentity {
            addrs: scanner.addrs.clone(),
            ..scanner_identity()
        },
    )
    .await
    .unwrap();
    let scanner_addr = {
        let scanned = issuer_state.scanned.lock().unwrap();
        addr_for(scanned[0].0, &scanned[0].1.addrs)
    };

    // 主人拒绝。
    deliver_decision(
        issuer.endpoint(),
        scanner_addr.clone(),
        false,
        issuer_state.own_identity(),
    )
    .await
    .unwrap();
    {
        let decisions = scanner_state.decisions.lock().unwrap();
        assert_eq!(decisions.len(), 1);
        assert!(!decisions[0].1, "应是拒绝");
    }

    // 删配对的通知。
    notify_removed(issuer.endpoint(), scanner_addr, "issuer.example")
        .await
        .unwrap();
    {
        let removed = scanner_state.removed.lock().unwrap();
        assert_eq!(removed.len(), 1);
        assert_eq!(removed[0].0, issuer.id, "通知的来源是握手认证过的对方");
        assert_eq!(removed[0].1, "issuer.example");
    }

    issuer.server.shutdown().await;
    scanner.server.shutdown().await;
}

/// 删除通知发到一台没有配对业务的 endpoint：协议层照样回 Ok，尽力而为。
#[tokio::test]
async fn a_removal_notice_to_a_server_without_any_record_still_succeeds() {
    let scanner_state = Arc::new(Scanner::default());
    let scanner = start(scanner_state.clone()).await;
    let stranger = loopback_endpoint(SecretKey::generate(), 0, Timeouts::Default).await;
    notify_removed(
        &stranger,
        addr_for(scanner.id, &scanner.addrs),
        "someone.example",
    )
    .await
    .unwrap();
    assert_eq!(scanner_state.removed.lock().unwrap().len(), 1);
    scanner.server.shutdown().await;
}

/// 出码方靠扫码请求里的地址回连。
#[tokio::test]
async fn the_issuer_dials_back_through_the_addresses_from_the_scan() {
    let issuer_state = Arc::new(Issuer::new());
    let issuer = start(issuer_state.clone()).await;
    let scanner = start(Arc::new(Scanner::default())).await;

    let code = code_for(&issuer, issuer_state.token.clone());
    let parsed = PeerPairingCode::parse(&code.to_uri()).unwrap();
    assert_eq!(parsed, code);
    assert_eq!(parsed.server_addr().id, issuer.id);

    let reply = scan_server(
        &scanner,
        &issuer,
        issuer_state.token.clone(),
        PeerIdentity {
            addrs: scanner.addrs.clone(),
            ..scanner_identity()
        },
    )
    .await
    .unwrap();
    assert!(matches!(reply, PeerReply::AwaitingApproval { .. }));
    {
        let scanned = issuer_state.scanned.lock().unwrap();
        assert_eq!(scanned[0].0, scanner.id, "扫码方身份来自 QUIC 握手");
        assert_eq!(scanned[0].1.addrs, scanner.addrs, "回连地址来自扫码请求");
    }
    issuer.server.shutdown().await;
    scanner.server.shutdown().await;
}

/// 码里的中继地址拼进连出码方的地址；没有直连地址。
#[tokio::test]
async fn the_code_address_holds_the_relay_only() {
    let issuer = start(Arc::new(Issuer::new())).await;
    let code = code_for(&issuer, PeerPairToken::from_bytes([7; 16]));
    let addr = code.server_addr();
    assert_eq!(addr.id, issuer.id);
    assert_eq!(
        addr.relay_urls().collect::<Vec<_>>(),
        vec![&"https://relay.example.com./".parse().unwrap()]
    );
    assert_eq!(addr.ip_addrs().count(), 0);
    issuer.server.shutdown().await;
}
