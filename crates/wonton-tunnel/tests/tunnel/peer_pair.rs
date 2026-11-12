//! 服务器配对的行为测试：两台“服务器”都在本机进程内，各自一个 endpoint 和服务器配对协议；
//! 配对业务（码表、记录）用测试里的替身处理方模拟。不依赖外网和 Docker。

use std::{
    net::SocketAddr,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use iroh::protocol::DynProtocolHandler;
use iroh::{EndpointId, SecretKey};
use wonton_tunnel::TunnelServer;
use wonton_tunnel::pair::{PairHandler, PairProtocol, PairResponse, RejectReason};
use wonton_tunnel::peer_pair::{
    PEER_PAIR_ALPN, PeerIdentity, PeerPairHandler, PeerPairProtocol, PeerPairSecret,
    PeerPairingCode, PeerRejectReason, PeerReply, deliver_decision, notify_removed, scan,
};

use crate::support::{Timeouts, loopback_endpoint};

/// 出码方的一次性码：有效期内能用一次。
#[derive(Debug)]
struct Issuer {
    server_name: String,
    secret: PeerPairSecret,
    expires_at: Mutex<Instant>,
    used: Mutex<bool>,
    /// 已配对的（endpoint ID，服务器名）。
    paired: Mutex<Vec<(EndpointId, String)>>,
    /// 扫进来的请求：扫码方的 endpoint ID、身份和能被回连的地址。
    scanned: Mutex<Vec<(EndpointId, PeerIdentity)>>,
    reject_self_name: bool,
}

impl PeerPairHandler for Issuer {
    fn scanned(
        &self,
        scanner: EndpointId,
        identity: &PeerIdentity,
        secret: &PeerPairSecret,
    ) -> PeerReply {
        if self.reject_self_name && identity.server_name == self.server_name {
            return PeerReply::Error {
                reason: PeerRejectReason::SelfPairing,
                message: "扫了自己的服务器".to_owned(),
            };
        }
        if *self.used.lock().unwrap() {
            return PeerReply::Error {
                reason: PeerRejectReason::Used,
                message: "这个码已经用过".to_owned(),
            };
        }
        if *secret != self.secret {
            return PeerReply::Error {
                reason: PeerRejectReason::UnknownCode,
                message: "没发过这个码".to_owned(),
            };
        }
        if Instant::now() >= *self.expires_at.lock().unwrap() {
            return PeerReply::Error {
                reason: PeerRejectReason::Expired,
                message: "码过期了".to_owned(),
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
        *self.used.lock().unwrap() = true;
        self.scanned
            .lock()
            .unwrap()
            .push((scanner, identity.clone()));
        PeerReply::AwaitingApproval {
            identity: PeerIdentity {
                server_name: self.server_name.clone(),
                owner_user_id: "@alex:issuer.example".to_owned(),
                owner_display_name: "Alex".to_owned(),
                relays: Vec::new(),
                addrs: Vec::new(),
            },
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
    fn scanned(&self, _: EndpointId, _: &PeerIdentity, _: &PeerPairSecret) -> PeerReply {
        PeerReply::Error {
            reason: PeerRejectReason::BadRequest,
            message: "扫码方不该收到扫码".to_owned(),
        }
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

fn code_for(issuer: &Server, secret: PeerPairSecret) -> PeerPairingCode {
    PeerPairingCode {
        server_id: issuer.id,
        relay_urls: Vec::new(),
        direct_addrs: issuer.addrs.clone(),
        server_name: "issuer.example".to_owned(),
        secret,
    }
}

fn scanner_identity() -> PeerIdentity {
    PeerIdentity {
        server_name: "scanner.example".to_owned(),
        owner_user_id: "@jiale:scanner.example".to_owned(),
        owner_display_name: "Jiale".to_owned(),
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

#[tokio::test]
async fn a_scan_gets_awaiting_approval_and_the_decision_reaches_only_the_scanned_endpoint() {
    let issuer_state = Arc::new(Issuer {
        server_name: "issuer.example".to_owned(),
        secret: PeerPairSecret::from_bytes([7; 16]),
        expires_at: Mutex::new(Instant::now() + Duration::from_secs(60)),
        used: Mutex::new(false),
        paired: Mutex::new(Vec::new()),
        scanned: Mutex::new(Vec::new()),
        reject_self_name: false,
    });
    let issuer = start(issuer_state.clone()).await;
    let scanner_state = Arc::new(Scanner::default());
    let scanner = start(scanner_state.clone()).await;

    // 扫码：出码方回了“等主人同意”，附自己的身份。
    let code = code_for(&issuer, PeerPairSecret::from_bytes([7; 16]));
    scanner_state.expecting.lock().unwrap().push(issuer.id);
    let reply = scan(
        scanner.endpoint(),
        code,
        PeerIdentity {
            server_name: "scanner.example".to_owned(),
            owner_user_id: "@jiale:scanner.example".to_owned(),
            owner_display_name: "Jiale".to_owned(),
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
        PeerIdentity {
            server_name: "issuer.example".to_owned(),
            owner_user_id: "@alex:issuer.example".to_owned(),
            owner_display_name: "Alex".to_owned(),
            relays: Vec::new(),
            addrs: Vec::new(),
        },
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

#[tokio::test]
async fn expired_used_tampered_and_self_codes_are_rejected_with_distinct_reasons() {
    let issuer_state = Arc::new(Issuer {
        server_name: "issuer.example".to_owned(),
        secret: PeerPairSecret::from_bytes([7; 16]),
        expires_at: Mutex::new(Instant::now() + Duration::from_secs(60)),
        used: Mutex::new(false),
        paired: Mutex::new(Vec::new()),
        scanned: Mutex::new(Vec::new()),
        reject_self_name: true,
    });
    let issuer = start(issuer_state.clone()).await;
    let dialer = loopback_endpoint(SecretKey::generate(), 0, Timeouts::Default).await;

    // 码被改：密钥对不上。
    let tampered = code_for(&issuer, PeerPairSecret::from_bytes([8; 16]));
    let reply = scan(&dialer, tampered, scanner_identity()).await.unwrap();
    assert_eq!(
        reply,
        PeerReply::Error {
            reason: PeerRejectReason::UnknownCode,
            message: "没发过这个码".to_owned(),
        }
    );

    // 扫自己：扫码方报的服务器名和出码方一样。
    let good = code_for(&issuer, PeerPairSecret::from_bytes([7; 16]));
    let mut self_scanner = scanner_identity();
    self_scanner.server_name = "issuer.example".to_owned();
    let reply = scan(&dialer, good.clone(), self_scanner).await.unwrap();
    assert!(matches!(
        reply,
        PeerReply::Error {
            reason: PeerRejectReason::SelfPairing,
            ..
        }
    ));

    // 码过期。
    *issuer_state.expires_at.lock().unwrap() = Instant::now() - Duration::from_secs(1);
    let reply = scan(&dialer, good, scanner_identity()).await.unwrap();
    assert!(matches!(
        reply,
        PeerReply::Error {
            reason: PeerRejectReason::Expired,
            ..
        }
    ));

    // 码已用过：同一张码第二次扫（替身只认自己的密钥，“新码”也是它）。
    *issuer_state.expires_at.lock().unwrap() = Instant::now() + Duration::from_secs(60);
    let fresh = code_for(&issuer, PeerPairSecret::from_bytes([7; 16]));
    let reply = scan(&dialer, fresh.clone(), scanner_identity())
        .await
        .unwrap();
    assert!(matches!(reply, PeerReply::AwaitingApproval { .. }));
    let reply = scan(&dialer, fresh, scanner_identity()).await.unwrap();
    assert!(matches!(
        reply,
        PeerReply::Error {
            reason: PeerRejectReason::Used,
            ..
        }
    ));

    // 已经配对过：码没用过的前提下，对方已在配对名单里。
    *issuer_state.used.lock().unwrap() = false;
    issuer_state
        .paired
        .lock()
        .unwrap()
        .push((SecretKey::generate().public(), "scanner.example".to_owned()));
    let fresh = code_for(&issuer, PeerPairSecret::from_bytes([7; 16]));
    let reply = scan(&dialer, fresh, scanner_identity()).await.unwrap();
    assert!(matches!(
        reply,
        PeerReply::Error {
            reason: PeerRejectReason::AlreadyPaired,
            ..
        }
    ));
    issuer.server.shutdown().await;
}

#[tokio::test]
async fn a_rejected_decision_and_a_removal_notice_reach_the_scanner() {
    let issuer_state = Arc::new(Issuer {
        server_name: "issuer.example".to_owned(),
        secret: PeerPairSecret::from_bytes([7; 16]),
        expires_at: Mutex::new(Instant::now() + Duration::from_secs(60)),
        used: Mutex::new(false),
        paired: Mutex::new(Vec::new()),
        scanned: Mutex::new(Vec::new()),
        reject_self_name: false,
    });
    let issuer = start(issuer_state.clone()).await;
    let scanner_state = Arc::new(Scanner::default());
    let scanner = start(scanner_state.clone()).await;

    let code = code_for(&issuer, PeerPairSecret::from_bytes([7; 16]));
    scanner_state.expecting.lock().unwrap().push(issuer.id);
    scan(
        scanner.endpoint(),
        code,
        PeerIdentity {
            server_name: "scanner.example".to_owned(),
            owner_user_id: "@jiale:scanner.example".to_owned(),
            owner_display_name: "Jiale".to_owned(),
            relays: Vec::new(),
            addrs: scanner.addrs.clone(),
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
        PeerIdentity {
            server_name: "issuer.example".to_owned(),
            owner_user_id: "@alex:issuer.example".to_owned(),
            owner_display_name: "Alex".to_owned(),
            relays: Vec::new(),
            addrs: Vec::new(),
        },
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

/// 码的地址拼法和回连地址一致：出码方靠扫码请求里的地址回连。
#[tokio::test]
async fn the_issuer_dials_back_through_the_addresses_from_the_scan() {
    let issuer_state = Arc::new(Issuer {
        server_name: "issuer.example".to_owned(),
        secret: PeerPairSecret::from_bytes([7; 16]),
        expires_at: Mutex::new(Instant::now() + Duration::from_secs(60)),
        used: Mutex::new(false),
        paired: Mutex::new(Vec::new()),
        scanned: Mutex::new(Vec::new()),
        reject_self_name: false,
    });
    let issuer = start(issuer_state.clone()).await;
    let scanner_state = Arc::new(Scanner::default());
    let scanner = start(scanner_state.clone()).await;

    let code = code_for(&issuer, PeerPairSecret::from_bytes([7; 16]));
    let parsed = PeerPairingCode::parse(&code.to_uri()).unwrap();
    assert_eq!(parsed, code);
    assert_eq!(parsed.server_addr().id, issuer.id);

    scanner_state.expecting.lock().unwrap().push(issuer.id);
    let reply = scan(
        scanner.endpoint(),
        code,
        PeerIdentity {
            server_name: "scanner.example".to_owned(),
            owner_user_id: "@jiale:scanner.example".to_owned(),
            owner_display_name: "Jiale".to_owned(),
            relays: Vec::new(),
            addrs: scanner.addrs.clone(),
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
