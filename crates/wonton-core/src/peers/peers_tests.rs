//! 服务器配对的行为测试：两台“核心”都在本机进程内，各自真实的 iroh endpoint 和服务器配对协议，
//! 私聊提醒由测试替身记下。覆盖长效码、请求、限频、拒绝和撤销、互相发请求、重名、同意之后两边成立、
//! 扫码方离线时的问结果、取消请求、删配对通知和重新配对。
//!
//! 删除相关的测试在 `peers_tests/removal.rs`：本机“收到删除通知”和“回来后握手被对方拒绝”两条路
//! 都按转发组件的接口模拟。接上真实联邦转发组件的测试在 `peers_tests/forwarder.rs`。

mod code;
mod forwarder;
mod harness;
mod removal;
mod requests;

use std::collections::{BTreeMap, BTreeSet};
use std::net::Ipv4Addr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::future::BoxFuture;
use iroh::endpoint::presets;
use iroh::{Endpoint, EndpointAddr, SecretKey};
use tokio::net::TcpListener;
use tokio::sync::watch;
use wonton_protocol::{PeerError, PeerRequestRow, PeerStatus, RejectedPeerRow};
use wonton_tunnel::TunnelServer;
use wonton_tunnel::federation::{
    FEDERATION_ALPN, FederationForwarder, FederationProtocol, PeerStates,
    PeerStatus as TunnelPeerStatus, ServerLookup,
};
use wonton_tunnel::peer_pair::{PEER_PAIR_ALPN, PeerIdentity, deliver_decision};
use wonton_tunnel::relays::RelayTable;

use super::{OwnIdentity, OwnerNotices, PeerRequest, Peers};
use crate::device::Devices;
use crate::plugin::languages::Languages;
use crate::tunnel::Tunnel;

/// 等待异步事件的上限。
const WAIT: Duration = Duration::from_secs(15);

/// 私聊里的提醒：测试里只记下内容，写入时叫醒等待者。
#[derive(Debug, Default)]
struct Notices {
    sent: Mutex<Vec<serde_json::Value>>,
    changed: tokio::sync::Notify,
}

impl OwnerNotices for Notices {
    fn send(&self, content: serde_json::Value) -> BoxFuture<'_, Result<(), String>> {
        Box::pin(async move {
            self.sent.lock().unwrap().push(content);
            self.changed.notify_waiters();
            Ok(())
        })
    }
}

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
    /// endpoint 的密钥：重启时要还是同一个 endpoint ID（配对按它认定对方）。
    key: SecretKey,
    /// 自己的身份：重启后还是同一个服务器名和主人。
    own: OwnIdentity,
    owner_name: Arc<Mutex<String>>,
    notices: Arc<Notices>,
    paired_names: watch::Receiver<BTreeSet<String>>,
    _server: Option<TunnelServer>,
    /// 联邦入站接到的上游：探活只回 Ping，不碰它。
    _upstream: TcpListener,
    forwarder: Option<FederationForwarder>,
    states: Option<watch::Receiver<PeerStates>>,
    dir: tempfile::TempDir,
}

impl TestCore {
    /// 起一台测试核心：隧道 endpoint + 服务器配对协议 + 联邦入站，没有 Matrix。主人的语言是中文。
    async fn start(name: &str, server_name: &str) -> Self {
        Self::start_in("zh", name, server_name).await
    }

    /// 同 [`Self::start`]，主人的语言由 `lang` 定（ADR-0043 的文字按它写）。
    async fn start_in(lang: &str, name: &str, server_name: &str) -> Self {
        Self::start_seeded(lang, name, server_name, Vec::new(), 0).await
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

    /// 等一个和记录有关的条件成立：写盘会推进版本号，不用隔一会儿问一次。
    async fn wait_for(&self, mut done: impl FnMut(&TestCore) -> bool, what: &str) {
        let mut changes = self.peers.changes();
        tokio::time::timeout(WAIT, async {
            loop {
                if done(self) {
                    return;
                }
                if changes.changed().await.is_err() {
                    return;
                }
            }
        })
        .await
        .unwrap_or_else(|_| panic!("等{what}超时"));
    }

    /// 等到这台核心把 `server_name` 记成已配对，返回绑定的对方 endpoint ID。
    async fn wait_paired(&self, server_name: &str) -> iroh::EndpointId {
        self.wait_for(|core| core.peers.lookup(server_name).is_some(), "配对成立")
            .await;
        self.peers.lookup(server_name).expect("刚等到的配对").0
    }

    /// 等到 `server_name` 的记录消失。
    async fn wait_gone(&self, server_name: &str) {
        self.wait_for(|core| core.peers.lookup(server_name).is_none(), "记录消失")
            .await;
    }

    /// 等到视图里 `server_name` 的行消失（没配对的记录 lookup 看不见，要看行）。
    async fn wait_row_gone(&self, server_name: &str) {
        self.wait_for(|core| core.row(server_name).is_none(), "视图行消失")
            .await;
    }

    /// 视图里某台服务器的行。
    fn row(&self, server_name: &str) -> Option<wonton_protocol::PeerRow> {
        self.peers
            .view(&langs())
            .peers
            .into_iter()
            .find(|row| row.server_name == server_name)
    }

    /// 扫码的回应是收下请求、落盘之后才回的：`scan` 一返回，请求就该在视图里。
    fn request_now(&self, server_name: &str) -> PeerRequestRow {
        self.request(server_name)
            .unwrap_or_else(|| panic!("扫码返回时 {server_name} 的请求已经收下"))
    }

    /// 视图里某台服务器发来的请求。
    fn request(&self, server_name: &str) -> Option<PeerRequestRow> {
        self.peers
            .view(&langs())
            .requests
            .into_iter()
            .find(|row| row.server_name == server_name)
    }

    /// 视图里某台服务器被拒绝的那条。
    fn rejected(&self, server_name: &str) -> Option<RejectedPeerRow> {
        self.peers
            .view(&langs())
            .rejected
            .into_iter()
            .find(|row| row.server_name == server_name)
    }

    /// 等这一行记下最近连上的时刻，返回它。
    async fn wait_connected_at(&self, server_name: &str) -> u64 {
        self.wait_for(
            |core| {
                core.row(server_name)
                    .and_then(|row| row.last_connected_ms)
                    .is_some()
            },
            "最后连上的时刻",
        )
        .await;
        self.row(server_name)
            .and_then(|row| row.last_connected_ms)
            .expect("刚等到的时刻")
    }

    /// 等 `server_name` 被标成“对方删了”：记录和名单都撤掉，查找不再认它。
    async fn wait_removed(&self, server_name: &str) {
        let mut names = self.paired_names.clone();
        let mut changes = self.peers.changes();
        tokio::time::timeout(WAIT, async {
            loop {
                let marked = self.row(server_name).is_some()
                    && self.peers.lookup(server_name).is_none()
                    && !names.borrow().contains(server_name);
                if marked {
                    return;
                }
                tokio::select! {
                    changed = changes.changed() => if changed.is_err() { return },
                    changed = names.changed() => if changed.is_err() { return },
                }
            }
        })
        .await
        .unwrap_or_else(|_| panic!("等 {server_name} 标成对方已断开超时"));
    }

    /// 等已配对名单里出现 `server_name`。
    async fn wait_paired_names(&self, server_name: &str) {
        let mut names = self.paired_names.clone();
        tokio::time::timeout(WAIT, async {
            loop {
                if names.borrow_and_update().contains(server_name) {
                    return;
                }
                if names.changed().await.is_err() {
                    return;
                }
            }
        })
        .await
        .expect("等已配对名单更新超时");
    }

    /// 等视图里某台服务器的行变成想要的状态。
    async fn wait_row_status(&self, server_name: &str, want: PeerStatus) {
        self.wait_for(
            |core| core.row(server_name).is_some_and(|row| row.status == want),
            "行变成想要的状态",
        )
        .await;
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

    /// 扫对方出的码：码里没有直连地址，测试直接连对方的套接字。
    async fn scan_code(
        &self,
        code: &wonton_protocol::PeerCode,
        issuer: &TestCore,
    ) -> Result<(), PeerError> {
        let parsed = wonton_tunnel::peer_pair::PeerPairingCode::parse(code.code.expose())
            .expect("测试的码总能解析");
        self.peers
            .scan_to(&parsed, issuer.addr(), self.peers.own_identity())
            .await
    }

    /// 以给定的身份扫对方的码（制造对不上的用户 ID 之类）。
    async fn scan_as(
        &self,
        code: &wonton_protocol::PeerCode,
        issuer: &TestCore,
        identity: PeerIdentity,
    ) -> Result<(), PeerError> {
        let parsed = wonton_tunnel::peer_pair::PeerPairingCode::parse(code.code.expose())
            .expect("测试的码总能解析");
        self.peers.scan_to(&parsed, issuer.addr(), identity).await
    }

    /// 主人同意某台服务器发来的请求。
    async fn approve(&self, server_name: &str) -> Result<(), PeerError> {
        let id = self.request(server_name).expect("这条请求要在").id;
        self.peers.approve(&id).await
    }

    /// 主人拒绝某台服务器发来的请求。
    async fn reject(&self, server_name: &str) -> Result<(), PeerError> {
        let id = self.request(server_name).expect("这条请求要在").id;
        self.peers.reject(&id).await
    }

    /// 主人撤销一次拒绝。
    async fn unreject(&self, server_name: &str) -> Result<(), PeerError> {
        let id = self.rejected(server_name).expect("这条拒绝要在").id;
        self.peers.unreject(&id).await
    }

    /// 私聊里记下的提醒。
    fn notices(&self) -> Vec<serde_json::Value> {
        self.notices.sent.lock().unwrap().clone()
    }

    /// 等到私聊里攒够 `count` 条提醒（每一条写入时都会叫醒）。
    async fn wait_notices(&self, count: usize) {
        tokio::time::timeout(WAIT, async {
            loop {
                if self.notices().len() >= count {
                    return;
                }
                // 先登记再查一遍：查完到睡下之间的那条也不会漏。
                let changed = self.notices.changed.notified();
                if self.notices().len() >= count {
                    return;
                }
                changed.await;
            }
        })
        .await
        .unwrap_or_else(|_| panic!("等第 {count} 条提醒超时"));
    }

    /// 这台核心的配对记录文件。
    fn store_path(&self) -> std::path::PathBuf {
        self.dir.path().join("core/peers.json")
    }

    /// 从同一份存储重新起一个 `Peers`（核心重启）：返回新的记录表和它的已配对名单。
    /// 核心重启：停掉旧的协议服务、换一个 `Peers` 读同一份存储（endpoint 密钥和身份不变）。
    /// 旧的 `Peers` 放开，免得两个实例一起写同一个文件。
    async fn reopen(&mut self) -> (Arc<Peers>, watch::Receiver<BTreeSet<String>>) {
        if let Some(server) = self._server.take() {
            server.shutdown().await;
        }
        let devices = Arc::new(Devices::open(self.dir.path().join("devices.json")).unwrap());
        let endpoint = loopback_endpoint(self.key.clone()).await;
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
            self.own.clone(),
            tunnel,
            Some(self.notices.clone()),
            Arc::new(|| "Alex".to_owned()),
            Arc::new(|| Languages::from_header(Some("zh"))),
        )
        .unwrap();
        let names = peers.paired_names();
        self.peers = peers.clone();
        (peers, names)
    }

    /// 手动跑一轮“扫码方来问结果”。
    async fn poll(&self, server_name: &str) {
        self.peers.poll_outcome(server_name).await;
    }
}

/// `a` 出码、`b` 扫码、`a` 的主人同意，两边都配对成立。
async fn pair(a: &TestCore, b: &TestCore) {
    let code = a.peers.issue_code();
    b.scan_code(&code, a).await.unwrap();
    a.request_now("b.example.internal");
    a.approve("b.example.internal").await.unwrap();
    a.wait_paired("b.example.internal").await;
    b.wait_paired("a.example.internal").await;
    a.wait_paired_names("b.example.internal").await;
    b.wait_paired_names("a.example.internal").await;
}

/// 全流程：出码 → 扫码（等对方同意）→ 出码方主人同意 → 两边记成已配对、名单更新、主人身份记上。
/// 同一张长效码还能被别的服务器扫。
#[tokio::test]
async fn scan_then_approval_pairs_both_sides() {
    let a = TestCore::start("Alex", "a.example.internal").await;
    let b = TestCore::start("Jiale", "b.example.internal").await;

    let code = a.peers.issue_code();
    b.scan_code(&code, &a).await.unwrap();

    // 扫码方先看到“等对方同意”。
    let row = b.row("a.example.internal").expect("扫码后应有这条记录");
    assert_eq!(row.status, PeerStatus::AwaitingApproval);
    assert_eq!(row.owner_display_name, "Alex");
    assert!(a.peers.lookup("b.example.internal").is_none(), "还没同意");

    // 出码方的请求里是扫码方自报的内容，私聊里来了一条带链接的提醒。
    let request = a.request_now("b.example.internal");
    assert_eq!(request.owner_display_name, "Jiale");
    assert_eq!(request.owner_user_id, "@owner:b.example.internal");
    assert!(
        a.peers.lookup("b.example.internal").is_none(),
        "同意之前不成立"
    );
    a.wait_notices(1).await;
    let notice = a.notices().remove(0)["body"].as_str().unwrap().to_owned();
    assert!(notice.contains("wonton://settings/peers"), "{notice}");
    assert!(notice.contains("Jiale"), "{notice}");

    // 主人同意：两边记成已配对。
    a.approve("b.example.internal").await.unwrap();
    let a_bound = a.wait_paired("b.example.internal").await;
    let b_bound = b.wait_paired("a.example.internal").await;
    let code_id = wonton_tunnel::peer_pair::PeerPairingCode::parse(code.code.expose())
        .unwrap()
        .server_id;
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
    assert!(
        a.peers.view(&langs()).requests.is_empty(),
        "处理完的请求不再列出"
    );

    // 已经配对过：再扫同一个服务器的码，直接说清。
    b.scan_code(&code, &a).await.unwrap_err();
    assert!(a.peers.view(&langs()).requests.is_empty());
}

/// 主人改了资料再出码，对方扫到的是新名字，列表里显示的也是新名字（不是核心启动时的旧名字）。
#[tokio::test]
async fn an_owner_rename_reaches_the_peer_as_the_current_name() {
    let a = TestCore::start("Alex", "a.example.internal").await;
    let b = TestCore::start("Jiale", "b.example.internal").await;

    a.set_owner_name("改名了");
    let code = a.peers.issue_code();
    b.scan_code(&code, &a).await.unwrap();
    let row = b.row("a.example.internal").expect("扫码后应有这条记录");
    assert_eq!(row.owner_display_name, "改名了");

    let request = a.request_now("b.example.internal");
    assert_eq!(request.owner_display_name, "Jiale");
    a.approve("b.example.internal").await.unwrap();
    b.wait_paired("a.example.internal").await;
    assert_eq!(
        b.row("a.example.internal").unwrap().owner_display_name,
        "改名了"
    );
}

/// 扫码方离线时同意：上线后来问一次就成立。
#[tokio::test]
async fn a_decision_sent_while_the_scanner_is_offline_is_picked_up_by_the_poll() {
    let a = TestCore::start("Alex", "a.example.internal").await;
    let b = TestCore::start("Jiale", "b.example.internal").await;

    let code = a.peers.issue_code();
    // b 不带回连地址：a 送结果送不到，只能等 b 来问。
    b.scan_as(
        &code,
        &a,
        PeerIdentity {
            addrs: Vec::new(),
            ..b.peers.own_identity()
        },
    )
    .await
    .unwrap();
    a.request_now("b.example.internal");
    a.approve("b.example.internal").await.unwrap();

    a.wait_paired("b.example.internal").await;
    assert_eq!(
        b.row("a.example.internal").unwrap().status,
        PeerStatus::AwaitingApproval,
        "结果没送到，扫码方还停在等"
    );

    b.poll("a.example.internal").await;
    b.wait_paired("a.example.internal").await;
    assert!(b.peers.server_allowlist().contains(&a.id_endpoint()));
}

/// 同名不同 endpoint：本机还留着这个名字的记录（包括“对方已断开连接”）就回“已经配对过”。
#[tokio::test]
async fn a_taken_server_name_is_refused() {
    let a = TestCore::start("Alex", "a.example.internal").await;
    let b = TestCore::start("Jiale", "b.example.internal").await;
    pair(&a, &b).await;

    // 另一个 endpoint 冒充同一个服务器名。
    let impostor = TestCore::start("Fake", "b.example.internal").await;
    let code = a.peers.issue_code();
    let error = impostor.scan_code(&code, &a).await.unwrap_err();
    assert_eq!(error, PeerError::AlreadyPaired);

    // 对方删了以后，冒充的还是配不进来：这个名字的记录还在（对方已断开连接）。
    b.peers.remove("a.example.internal").await.unwrap();
    a.wait_removed("b.example.internal").await;
    let code = a.peers.issue_code();
    let error = impostor.scan_code(&code, &a).await.unwrap_err();
    assert_eq!(error, PeerError::AlreadyPaired);
}

/// 删配对：两边名单各自撤掉，对方收到通知后留在列表里，标成“对方已断开连接”。
#[tokio::test]
async fn removing_a_pairing_notifies_the_peer() {
    let a = TestCore::start("Alex", "a.example.internal").await;
    let b = TestCore::start("Jiale", "b.example.internal").await;
    pair(&a, &b).await;

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
    let a = TestCore::start("Alex", "a.example.internal").await;
    let b = TestCore::start("Jiale", "b.example.internal").await;

    assert!(a.peers.lookup("b.example.internal").is_none());
    let code = a.peers.issue_code();
    b.scan_code(&code, &a).await.unwrap();
    a.request_now("b.example.internal");
    a.approve("b.example.internal").await.unwrap();
    let bound = a.wait_paired("b.example.internal").await;
    assert_eq!(
        a.peers.lookup("b.example.internal").unwrap().0,
        bound,
        "绑定的就是扫码时的 endpoint ID"
    );
    a.peers.remove("b.example.internal").await.unwrap();
    assert!(a.peers.lookup("b.example.internal").is_none());
}

/// 请求和等对方的记录都不过期：重启后都还在。
#[tokio::test]
async fn a_restart_keeps_pending_requests_and_pairings() {
    let mut a = TestCore::start("Alex", "a.example.internal").await;
    let b = TestCore::start("Jiale", "b.example.internal").await;

    let code = a.peers.issue_code();
    b.scan_code(&code, &a).await.unwrap();
    a.request_now("b.example.internal");

    let (reopened, _) = a.reopen().await;
    assert_eq!(reopened.view(&langs()).requests.len(), 1);
    assert_eq!(
        reopened.view(&langs()).requests[0].server_name,
        "b.example.internal"
    );
}

/// 扫码之后核心重启：记录留着，启动时自己问一次结果，对方同意后就成立。
#[tokio::test]
async fn a_pending_scan_is_polled_again_after_a_restart() {
    let a = TestCore::start("Alex", "a.example.internal").await;
    let mut b = TestCore::start("Jiale", "b.example.internal").await;

    let code = a.peers.issue_code();
    b.scan_code(&code, &a).await.unwrap();
    a.request_now("b.example.internal");

    // 核心重启（同一份记录），主人这时才同意。
    let (reopened, names) = b.reopen().await;
    a.approve("b.example.internal").await.unwrap();
    a.wait_paired("b.example.internal").await;
    let mut changes = reopened.changes();
    tokio::time::timeout(WAIT, async {
        loop {
            if reopened.lookup("a.example.internal").is_some() {
                return;
            }
            if changes.changed().await.is_err() {
                return;
            }
        }
    })
    .await
    .expect("重启后要自己问出结果");
    let mut names = names;
    tokio::time::timeout(WAIT, async {
        loop {
            if names.borrow_and_update().contains("a.example.internal") {
                return;
            }
            if names.changed().await.is_err() {
                return;
            }
        }
    })
    .await
    .expect("结果送到以后要开始探活");
}

/// 等对方的记录不过期，视图照旧列出。
#[tokio::test]
async fn pending_rows_do_not_expire() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("peers.json");
    let waiting = super::PeerRecord {
        server_name: "late.example.internal".to_owned(),
        endpoint_id: SecretKey::generate().public(),
        relays: Vec::new(),
        addrs: Vec::new(),
        owner_user_id: "@owner:late.example.internal".to_owned(),
        owner_display_name: "Late".to_owned(),
        paired_at_ms: 0,
        phase: super::Phase::AwaitingPeer,
        last_connected_ms: None,
    };
    std::fs::write(
        &path,
        serde_json::to_string(&super::Stored {
            peers: BTreeMap::from([(waiting.server_name.clone(), waiting)]),
            ..Default::default()
        })
        .unwrap(),
    )
    .unwrap();
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
        None,
        Arc::new(|| "Alex".to_owned()),
        Arc::new(|| Languages::from_header(Some("zh"))),
    )
    .unwrap();
    let row = peers
        .view(&langs())
        .peers
        .into_iter()
        .find(|row| row.server_name == "late.example.internal")
        .expect("还在等的记录照旧列出");
    assert_eq!(row.status, PeerStatus::AwaitingApproval);
}
