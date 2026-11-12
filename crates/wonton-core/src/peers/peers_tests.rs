//! 服务器配对的行为测试：两台“核心”都在本机进程内，各自真实的 iroh endpoint 和服务器配对协议，
//! 确认卡片由测试替身通道记下、在进程内回答。覆盖出码 → 扫码 → 卡片同意 → 两边记成已配对的全流程，
//! 以及拒绝、过期、码被改、扫自己、已配对、结果只认扫过的 endpoint ID、删配对通知和重新配对。
//!
//! 删除相关的测试在 `peers_tests/removal.rs`：本机“收到删除通知”和“回来后握手被对方拒绝”两条路
//! 都按转发组件的接口模拟。接上真实联邦转发组件的测试在 `peers_tests/forwarder.rs`。

mod forwarder;
mod removal;

use std::collections::{BTreeMap, BTreeSet};
use std::net::Ipv4Addr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use iroh::endpoint::presets;
use iroh::{Endpoint, EndpointAddr, SecretKey};
use tokio::net::TcpListener;
use tokio::sync::watch;
use wonton_protocol::{Answer, PeerError, PeerStatus};
use wonton_tunnel::TunnelServer;
use wonton_tunnel::federation::{
    FEDERATION_ALPN, FederationForwarder, FederationProtocol, PeerStates,
    PeerStatus as TunnelPeerStatus, ServerLookup,
};
use wonton_tunnel::peer_pair::{PEER_PAIR_ALPN, deliver_decision};
use wonton_tunnel::relays::RelayTable;

use super::{OwnIdentity, Peers};
use crate::device::Devices;
use crate::mcp::Confirmations;
use crate::mcp::test_support::Cards;
use crate::plugin::languages::Languages;
use crate::tunnel::Tunnel;

/// 等待异步事件的上限。
const WAIT: Duration = Duration::from_secs(15);

/// 卡片等回答的时间：测试里要短的用 1 秒（过期测试），其余默认。
const SHORT_TTL: Duration = Duration::from_secs(1);

fn langs() -> Languages {
    Languages::from_header(Some("zh"))
}

async fn loopback_endpoint(key: SecretKey) -> Endpoint {
    Endpoint::builder(presets::Minimal)
        .secret_key(key)
        .clear_ip_transports()
        .bind_addr((Ipv4Addr::LOCALHOST, 0))
        .unwrap()
        .bind()
        .await
        .unwrap()
}

struct TestCore {
    peers: Arc<Peers>,
    owner_name: Arc<Mutex<String>>,
    confirmations: Arc<Confirmations>,
    cards: Arc<Cards>,
    paired_names: watch::Receiver<BTreeSet<String>>,
    _server: TunnelServer,
    /// 联邦入站接到的上游：探活只回 Ping，不碰它。
    _upstream: TcpListener,
    forwarder: Option<FederationForwarder>,
    states: Option<watch::Receiver<PeerStates>>,
    dir: tempfile::TempDir,
}

impl TestCore {
    /// 起一台测试核心：隧道 endpoint + 服务器配对协议 + 联邦入站，没有 Matrix。
    async fn start(name: &str, server_name: &str, ttl: Duration) -> Self {
        let cards = Arc::new(Cards::default());
        let confirmations = Arc::new(Confirmations::new(cards.clone(), ttl));
        let dir = tempfile::tempdir().unwrap();
        let devices = Arc::new(Devices::open(dir.path().join("devices.json")).unwrap());
        let endpoint = loopback_endpoint(SecretKey::generate()).await;
        let tunnel = Tunnel::new(
            devices,
            Arc::new(|| crate::plugin::languages::Languages::from_header(None)),
            endpoint,
            0,
            RelayTable::builtin_n0(),
            watch::channel(BTreeSet::from(["127.0.0.1".parse().unwrap()])).1,
        );
        let owner_name = Arc::new(Mutex::new(name.to_owned()));
        let peers = Peers::open(
            dir.path().join("core/peers.json"),
            OwnIdentity {
                server_name: server_name.to_owned(),
                owner_user_id: format!("@owner:{server_name}"),
            },
            tunnel,
            confirmations.clone(),
            Arc::new({
                let owner_name = owner_name.clone();
                move || owner_name.lock().unwrap().clone()
            }),
            Arc::new(|| Languages::from_header(Some("zh"))),
        )
        .unwrap();
        let paired_names = peers.paired_names();
        let upstream = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let federation = FederationProtocol::new(
            peers.server_allowlist().clone(),
            upstream.local_addr().unwrap(),
        );
        let (_server, _listener) = wonton_tunnel::TunnelServer::spawn_embedded_with_pairing_and(
            peers.tunnel().endpoint.clone(),
            peers.tunnel().devices.allowlist().clone(),
            wonton_tunnel::pair::PairProtocol::new(peers.tunnel().pairing.clone()),
            [
                (
                    PEER_PAIR_ALPN,
                    Box::new(wonton_tunnel::peer_pair::PeerPairProtocol::new(Arc::new(
                        super::pairing::PeerPairing(peers.clone()),
                    ))) as Box<dyn iroh::protocol::DynProtocolHandler>,
                ),
                (
                    FEDERATION_ALPN,
                    Box::new(federation) as Box<dyn iroh::protocol::DynProtocolHandler>,
                ),
            ],
        );
        Self {
            peers,
            owner_name,
            confirmations,
            cards,
            paired_names,
            _server,
            _upstream: upstream,
            forwarder: None,
            states: None,
            dir,
        }
    }

    /// 接上真实的联邦转发组件：出站探活走真的 iroh 连接，状态写进这台核心。
    async fn enable_federation(&mut self) {
        let lookup: ServerLookup = {
            let peers = self.peers.clone();
            Arc::new(move |name: &str| peers.lookup(name))
        };
        let forwarder = FederationForwarder::bind(
            self.peers.tunnel().endpoint.clone(),
            (Ipv4Addr::LOCALHOST, 0).into(),
            lookup,
            self.peers.paired_names(),
            Duration::from_millis(200),
            Duration::from_secs(2),
        )
        .await
        .unwrap();
        let states = forwarder.watch_status();
        self.peers.attach_forwarder(states.clone());
        self.states = Some(states);
        self.forwarder = Some(forwarder);
    }

    /// 把转发组件状态表里每一次出现的新状态记下来（按服务器名）。
    fn record_states(&self) -> Arc<Mutex<Vec<(String, TunnelPeerStatus)>>> {
        let history = Arc::new(Mutex::new(Vec::new()));
        let mut states = self.states.clone().expect("要先接上转发组件");
        let seen = history.clone();
        tokio::spawn(async move {
            while states.changed().await.is_ok() {
                for (name, status) in states.borrow_and_update().iter() {
                    seen.lock().unwrap().push((name.clone(), status.clone()));
                }
            }
        });
        history
    }

    /// 主人改资料。
    fn set_owner_name(&self, name: &str) {
        *self.owner_name.lock().unwrap() = name.to_owned();
    }

    /// 等到这台核心把 `server_name` 记成已配对，返回绑定的对方 endpoint ID。
    async fn wait_paired(&self, server_name: &str) -> iroh::EndpointId {
        tokio::time::timeout(WAIT, async {
            loop {
                if let Some((id, _)) = self.peers.lookup(server_name) {
                    return id;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("等配对成立超时")
    }

    /// 等到 `server_name` 的记录消失。
    async fn wait_gone(&self, server_name: &str) {
        tokio::time::timeout(WAIT, async {
            loop {
                if self.peers.lookup(server_name).is_none() {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("等记录消失超时");
    }

    /// 等到视图里 `server_name` 的行消失（没配对的记录 lookup 看不见，要看行）。
    async fn wait_row_gone(&self, server_name: &str) {
        tokio::time::timeout(WAIT, async {
            loop {
                if self.row(server_name).is_none() {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("等视图行消失超时");
    }

    /// 视图里某台服务器的行。
    fn row(&self, server_name: &str) -> Option<wonton_protocol::PeerRow> {
        self.peers
            .view(&langs())
            .peers
            .into_iter()
            .find(|row| row.server_name == server_name)
    }

    /// 等这一行记下最近连上的时刻，返回它。
    async fn wait_connected_at(&self, server_name: &str) -> u64 {
        tokio::time::timeout(WAIT, async {
            loop {
                if let Some(at) = self.row(server_name).and_then(|row| row.last_connected_ms) {
                    return at;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("等最后连上的时刻超时")
    }

    /// 等 `server_name` 被标成“对方删了”：记录和名单都撤掉，查找不再认它。
    async fn wait_removed(&self, server_name: &str) {
        tokio::time::timeout(WAIT, async {
            loop {
                let marked = self.row(server_name).is_some()
                    && self.peers.lookup(server_name).is_none()
                    && !self.paired_names.borrow().contains(server_name);
                if marked {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap_or_else(|_| panic!("等 {server_name} 标成对方已断开超时"));
    }

    /// 等已配对名单里出现 `server_name`。
    async fn wait_paired_names(&self, server_name: &str) {
        tokio::time::timeout(WAIT, async {
            loop {
                if self.paired_names.borrow().contains(server_name) {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("等已配对名单更新超时");
    }

    /// 等视图里某台服务器的行变成想要的状态。
    async fn wait_row_status(&self, server_name: &str, want: PeerStatus) {
        tokio::time::timeout(WAIT, async {
            loop {
                if self.row(server_name).is_some_and(|row| row.status == want) {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap_or_else(|_| panic!("等 {server_name} 变成 {want:?} 超时"));
    }

    /// 这台核心的 endpoint ID（配对码里带的那个）。
    fn id_endpoint(&self) -> iroh::EndpointId {
        self.peers.tunnel().endpoint.id()
    }

    /// 连这台核心用的地址：endpoint ID 加上它的直连地址（测试不开中继）。
    fn addr(&self) -> EndpointAddr {
        let mut addr = EndpointAddr::new(self.id_endpoint());
        for ip in self.peers.tunnel().direct_addrs() {
            addr = addr.with_ip_addr(ip);
        }
        addr
    }

    /// 这台核心的配对记录文件。
    fn store_path(&self) -> std::path::PathBuf {
        self.dir.path().join("core/peers.json")
    }

    /// 从同一份存储重新起一个 `Peers`（核心重启）：返回新的记录表和它的已配对名单。
    async fn reopen(&self) -> (Arc<Peers>, watch::Receiver<BTreeSet<String>>) {
        let devices = Arc::new(Devices::open(self.dir.path().join("devices.json")).unwrap());
        let endpoint = loopback_endpoint(SecretKey::generate()).await;
        let tunnel = Tunnel::new(
            devices,
            Arc::new(|| Languages::from_header(None)),
            endpoint,
            0,
            RelayTable::builtin_n0(),
            watch::channel(BTreeSet::from(["127.0.0.1".parse().unwrap()])).1,
        );
        let peers = Peers::open(
            self.store_path(),
            OwnIdentity {
                server_name: "a.example.internal".to_owned(),
                owner_user_id: "@owner:a.example.internal".to_owned(),
            },
            tunnel,
            self.confirmations.clone(),
            Arc::new(|| "Alex".to_owned()),
            Arc::new(|| Languages::from_header(Some("zh"))),
        )
        .unwrap();
        let names = peers.paired_names();
        (peers, names)
    }
}

/// `a` 出码、`b` 扫码，`a` 的主人同意，两边都配对成立。
async fn pair(a: &TestCore, b: &TestCore) {
    let code = a.peers.issue_code();
    b.peers.scan(code.code.expose()).await.unwrap();
    let card = a.cards.wait_for(1).await;
    a.confirmations
        .answer(&card.id, Answer::Approve, None)
        .unwrap();
    a.wait_paired("b.example.internal").await;
    b.wait_paired("a.example.internal").await;
    a.wait_paired_names("b.example.internal").await;
    b.wait_paired_names("a.example.internal").await;
}

fn tamper_secret(uri: &str) -> String {
    let mut url = url::Url::parse(uri).unwrap();
    let pairs: Vec<(String, String)> = url
        .query_pairs()
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();
    url.query_pairs_mut()
        .clear()
        .extend_pairs(pairs.iter().map(|(k, v)| {
            if k == "secret" {
                (k.clone(), "ffffffffffffffffffffffffffffffff".to_owned())
            } else {
                (k.clone(), v.clone())
            }
        }));
    url.into()
}

/// 全流程：出码 → 扫码（等对方同意）→ 出码方主人同意 → 两边记成已配对、名单更新、主人身份记上。
#[tokio::test]
async fn scan_then_approval_pairs_both_sides() {
    let a = TestCore::start("Alex", "a.example.internal", Duration::from_secs(60)).await;
    let b = TestCore::start("Jiale", "b.example.internal", Duration::from_secs(60)).await;

    let code = a.peers.issue_code();
    b.peers.scan(code.code.expose()).await.unwrap();

    // 扫码方先看到“等对方同意”。
    let row = b.row("a.example.internal").expect("扫码后应有这条记录");
    assert_eq!(row.status, PeerStatus::AwaitingApproval);
    assert_eq!(row.owner_display_name, "Alex");
    assert!(a.peers.lookup("b.example.internal").is_none(), "还没同意");

    // 出码方的卡片带上了配对请求的内容。
    let card = a.cards.wait_for(1).await;
    let request = card.peer_request.expect("应是服务器配对请求的卡片");
    assert_eq!(request.server_name, "b.example.internal");
    assert_eq!(request.owner_display_name, "Jiale");
    assert!(
        card.title.contains("Jiale"),
        "标题要说清是谁的服务器：{}",
        card.title
    );

    // 主人同意：两边记成已配对。
    a.confirmations
        .answer(&card.id, Answer::Approve, None)
        .unwrap();
    let a_bound = a.wait_paired("b.example.internal").await;
    let b_bound = b.wait_paired("a.example.internal").await;
    let _ = (&a_bound, &b_bound);

    // 绑定就是码里的 endpoint ID。
    let code_id = {
        let parsed = wonton_tunnel::peer_pair::PeerPairingCode::parse(code.code.expose()).unwrap();
        parsed.server_id
    };
    assert_eq!(a_bound, b.id_endpoint());
    assert_eq!(b_bound, code_id);

    // 名单跟着更新（结果送出去以后），行带上主人的身份。
    a.wait_paired_names("b.example.internal").await;
    b.wait_paired_names("a.example.internal").await;
    let row = a.row("b.example.internal").unwrap();
    assert_eq!(row.owner_display_name, "Jiale");
    assert_eq!(row.owner_user_id, "@owner:b.example.internal");
    assert_eq!(
        row.status,
        PeerStatus::Checking,
        "没接联邦转发组件时状态还在确认"
    );

    // 已经配对过：再扫同一个服务器的码，直接说清。
    let code = a.peers.issue_code();
    let error = b.peers.scan(code.code.expose()).await.unwrap_err();
    assert_eq!(error, PeerError::AlreadyPaired);
}

/// 主人改了资料再出码，对方扫到的是新名字，列表里显示的也是新名字（不是核心启动时的旧名字）。
#[tokio::test]
async fn an_owner_rename_reaches_the_peer_as_the_current_name() {
    let a = TestCore::start("Alex", "a.example.internal", Duration::from_secs(60)).await;
    let b = TestCore::start("Jiale", "b.example.internal", Duration::from_secs(60)).await;

    a.set_owner_name("改名了");
    let code = a.peers.issue_code();
    b.peers.scan(code.code.expose()).await.unwrap();
    let row = b.row("a.example.internal").expect("扫码后应有这条记录");
    assert_eq!(row.owner_display_name, "改名了");

    let card = a.cards.wait_for(1).await;
    assert_eq!(
        card.peer_request
            .expect("应是配对请求的卡片")
            .owner_display_name,
        "Jiale"
    );
    a.confirmations
        .answer(&card.id, Answer::Approve, None)
        .unwrap();
    b.wait_paired("a.example.internal").await;
    assert_eq!(
        b.row("a.example.internal").unwrap().owner_display_name,
        "改名了"
    );
}

/// 主人拒绝：出码方的记录删掉，扫码方收到结果后也删掉。
#[tokio::test]
async fn a_rejected_card_removes_the_record_on_both_sides() {
    let a = TestCore::start("Alex", "a.example.internal", Duration::from_secs(60)).await;
    let b = TestCore::start("Jiale", "b.example.internal", Duration::from_secs(60)).await;

    let code = a.peers.issue_code();
    b.peers.scan(code.code.expose()).await.unwrap();
    let card = a.cards.wait_for(1).await;
    a.confirmations
        .answer(&card.id, Answer::Reject, None)
        .unwrap();

    // 扫码方的行随“拒绝”的结果消失；出码方这边记录由拒绝直接删掉。
    b.wait_row_gone("a.example.internal").await;
    assert!(!a.paired_names.borrow().contains("b.example.internal"));
    assert!(!b.paired_names.borrow().contains("a.example.internal"));
}

/// 卡片过期等于拒绝：出码方的记录删掉，扫码方收到拒绝。
#[tokio::test]
async fn an_expired_card_counts_as_a_rejection() {
    let a = TestCore::start("Alex", "a.example.internal", SHORT_TTL).await;
    let b = TestCore::start("Jiale", "b.example.internal", Duration::from_secs(60)).await;

    let code = a.peers.issue_code();
    b.peers.scan(code.code.expose()).await.unwrap();
    // 没人回答，卡片过期：出码方删记录，扫码方收到拒绝后也删。
    b.wait_row_gone("a.example.internal").await;
}

/// 码过期：出码方按 `code_expired` 拒绝。
#[tokio::test]
async fn an_expired_code_is_rejected_with_a_distinct_error() {
    let a = TestCore::start("Alex", "a.example.internal", Duration::from_secs(60)).await;
    let b = TestCore::start("Jiale", "b.example.internal", Duration::from_secs(60)).await;

    let code = a.peers.issue_code_expired_for_test();
    let error = b.peers.scan(code.code.expose()).await.unwrap_err();
    assert_eq!(error, PeerError::CodeExpired);
    assert!(b.row("a.example.internal").is_none());
}

/// 码被改：密钥对不上，按 `code_invalid` 拒绝。
#[tokio::test]
async fn a_tampered_code_is_rejected_as_invalid() {
    let a = TestCore::start("Alex", "a.example.internal", Duration::from_secs(60)).await;
    let b = TestCore::start("Jiale", "b.example.internal", Duration::from_secs(60)).await;

    let code = a.peers.issue_code();
    let error = b
        .peers
        .scan(&tamper_secret(code.code.expose()))
        .await
        .unwrap_err();
    assert_eq!(error, PeerError::CodeInvalid);
    assert!(b.row("a.example.internal").is_none());
}

/// 扫自己：不用连，直接说清。
#[tokio::test]
async fn scanning_ones_own_code_is_rejected() {
    let a = TestCore::start("Alex", "a.example.internal", Duration::from_secs(60)).await;
    let code = a.peers.issue_code();
    let error = a.peers.scan(code.code.expose()).await.unwrap_err();
    assert_eq!(error, PeerError::SelfPairing);
}

/// 配对结果只认自己扫过的那个 endpoint ID：别的 endpoint 冒充出码方送结果，扫码方不理。
#[tokio::test]
async fn a_decision_from_an_unscanned_endpoint_is_ignored() {
    let a = TestCore::start("Alex", "a.example.internal", Duration::from_secs(60)).await;
    let b = TestCore::start("Jiale", "b.example.internal", Duration::from_secs(60)).await;

    let code = a.peers.issue_code();
    b.peers.scan(code.code.expose()).await.unwrap();
    let card = a.cards.wait_for(1).await;
    let _ = card;

    // 冒充者：a 出码方的码已经用过，冒充者拿自己的 endpoint 连 b。
    let impostor = loopback_endpoint(SecretKey::generate()).await;
    let error = deliver_decision(
        &impostor,
        iroh::EndpointAddr::new(b.id_endpoint()),
        true,
        wonton_tunnel::peer_pair::PeerIdentity {
            server_name: "impostor.example.internal".to_owned(),
            owner_user_id: "@evil:impostor.example.internal".to_owned(),
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
    assert!(b.peers.lookup("impostor.example.internal").is_none());
    assert!(b.peers.lookup("a.example.internal").is_none(), "没配上");
}

/// 删配对：两边名单各自撤掉，对方收到通知后留在列表里，标成“对方已断开连接”。
#[tokio::test]
async fn removing_a_pairing_notifies_the_peer() {
    let a = TestCore::start("Alex", "a.example.internal", Duration::from_secs(60)).await;
    let b = TestCore::start("Jiale", "b.example.internal", Duration::from_secs(60)).await;

    let code = a.peers.issue_code();
    b.peers.scan(code.code.expose()).await.unwrap();
    let card = a.cards.wait_for(1).await;
    a.confirmations
        .answer(&card.id, Answer::Approve, None)
        .unwrap();
    a.wait_paired("b.example.internal").await;
    b.wait_paired("a.example.internal").await;

    assert!(a.peers.remove("b.example.internal").await.unwrap());
    a.wait_gone("b.example.internal").await;
    // 被删的一方留着记录，只是标成“对方删了”：联邦名单和查找都不再认它。
    b.wait_removed("a.example.internal").await;
    assert!(!a.paired_names.borrow().contains("b.example.internal"));
    assert!(b.row("a.example.internal").is_some());

    // 再删一次：没有这条记录了。
    assert!(!a.peers.remove("b.example.internal").await.unwrap());
}

/// 查找函数的规则：一个服务器名只会解析到配对时绑定的 endpoint ID，没配对和删掉之后都查不到。
#[tokio::test]
async fn a_server_name_resolves_only_to_its_bound_endpoint() {
    let a = TestCore::start("Alex", "a.example.internal", Duration::from_secs(60)).await;
    let b = TestCore::start("Jiale", "b.example.internal", Duration::from_secs(60)).await;

    assert!(a.peers.lookup("b.example.internal").is_none());
    let code = a.peers.issue_code();
    b.peers.scan(code.code.expose()).await.unwrap();
    let card = a.cards.wait_for(1).await;
    a.confirmations
        .answer(&card.id, Answer::Approve, None)
        .unwrap();
    let bound = a.wait_paired("b.example.internal").await;
    assert_eq!(
        a.peers.lookup("b.example.internal").unwrap().0,
        bound,
        "绑定的就是配对时码里的 endpoint ID"
    );
    a.peers.remove("b.example.internal").await.unwrap();
    assert!(a.peers.lookup("b.example.internal").is_none());
}

/// 重启：还在等我们回答的记录不会留下来（卡片没有留下，回答永远不会来），已配对的原样读回。
#[tokio::test]
async fn a_restart_drops_unanswered_requests_and_keeps_pairings() {
    let a = TestCore::start("Alex", "a.example.internal", Duration::from_secs(60)).await;
    let b = TestCore::start("Jiale", "b.example.internal", Duration::from_secs(60)).await;

    let code = a.peers.issue_code();
    b.peers.scan(code.code.expose()).await.unwrap();
    let card = a.cards.wait_for(1).await;
    a.confirmations
        .answer(&card.id, Answer::Approve, None)
        .unwrap();
    let bound = a.wait_paired("b.example.internal").await;

    // 另一个人扫了 a 的码，还没回答，这时核心重启。
    let c = TestCore::start("Chen", "c.example.internal", Duration::from_secs(60)).await;
    let second = a.peers.issue_code();
    c.peers.scan(second.code.expose()).await.unwrap();
    let _card = a.cards.wait_for(2).await;
    let path = a.dir.path().join("core/peers.json");
    let confirmations = a.confirmations.clone();
    let tunnel_clone = {
        let devices = Arc::new(Devices::open(a.dir.path().join("devices.json")).unwrap());
        let endpoint = loopback_endpoint(SecretKey::generate()).await;
        Tunnel::new(
            devices,
            Arc::new(|| Languages::from_header(None)),
            endpoint,
            0,
            RelayTable::builtin_n0(),
            watch::channel(BTreeSet::from(["127.0.0.1".parse().unwrap()])).1,
        )
    };
    let languages: Arc<dyn Fn() -> Languages + Send + Sync> =
        Arc::new(|| Languages::from_header(Some("zh")));
    let reopened = Peers::open(
        path,
        OwnIdentity {
            server_name: "a.example.internal".to_owned(),
            owner_user_id: "@owner:a.example.internal".to_owned(),
        },
        tunnel_clone,
        confirmations,
        Arc::new(|| "Alex".to_owned()),
        languages,
    )
    .unwrap();
    assert_eq!(reopened.lookup("b.example.internal").unwrap().0, bound);
    assert!(reopened.lookup("c.example.internal").is_none());
}

/// 过了期的“等对方同意”就地删掉，不出现在视图里。
#[tokio::test]
async fn expired_pending_rows_are_dropped_from_the_view() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("peers.json");
    let expired = super::PeerRecord {
        server_name: "late.example.internal".to_owned(),
        endpoint_id: SecretKey::generate().public(),
        relays: Vec::new(),
        addrs: Vec::new(),
        owner_user_id: "@owner:late.example.internal".to_owned(),
        owner_display_name: "Late".to_owned(),
        paired_at_ms: 0,
        phase: super::Phase::AwaitingPeer,
        last_connected_ms: None,
        pending_expires_at_ms: Some(1),
    };
    std::fs::write(
        &path,
        serde_json::to_string(&super::Stored {
            peers: vec![expired],
        })
        .unwrap(),
    )
    .unwrap();
    let confirmations = Arc::new(Confirmations::new(
        Arc::new(Cards::default()),
        Duration::from_secs(60),
    ));
    let devices = Arc::new(Devices::open(dir.path().join("devices.json")).unwrap());
    let endpoint = loopback_endpoint(SecretKey::generate()).await;
    let tunnel = Tunnel::new(
        devices,
        Arc::new(|| Languages::from_header(None)),
        endpoint,
        0,
        RelayTable::builtin_n0(),
        watch::channel(BTreeSet::from(["127.0.0.1".parse().unwrap()])).1,
    );
    let peers = Peers::open(
        &path,
        OwnIdentity {
            server_name: "a.example.internal".to_owned(),
            owner_user_id: "@owner:a.example.internal".to_owned(),
        },
        tunnel,
        confirmations,
        Arc::new(|| "Alex".to_owned()),
        Arc::new(|| Languages::from_header(Some("zh"))),
    )
    .unwrap();
    assert!(
        peers
            .view(&langs())
            .peers
            .iter()
            .all(|row| row.server_name != "late.example.internal"),
        "过了期的记录不再显示"
    );
}
