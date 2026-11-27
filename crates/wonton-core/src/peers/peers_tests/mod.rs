//! 服务器配对的行为测试：两台“核心”都在本机进程内，各自真实的 iroh endpoint 和服务器配对协议，
//! 私聊提醒由测试替身记下。覆盖长效码、请求、限频、拒绝和撤销、互相发请求、重名、同意之后两边成立、
//! 扫码方离线时的问结果、取消请求、删配对通知和重新配对。
//!
//! 删除相关的测试在 `peers_tests/removal.rs`：本机“收到删除通知”和“回来后握手被对方拒绝”两条路
//! 都按转发组件的接口模拟。接上真实联邦转发组件的测试在 `peers_tests/forwarder.rs`。配对本身在
//! `peers_tests/pairing.rs`，重启和记录不过期在 `peers_tests/persistence.rs`，证书钉住在 `peers_tests/pins.rs`。

mod code;
mod forwarder;
mod harness;
mod pairing;
mod persistence;
mod public_access;
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
use wonton_tunnel::peer_pair::{PEER_PAIR_ALPN, PeerCertificate, PeerIdentity, deliver_decision};
use wonton_tunnel::relays::RelayTable;

use super::{OwnIdentity, OwnerNotices, PeerRequest, Peers, PinState};
pub(super) use super::{PeerRecord, Phase, Stored};
use crate::device::Devices;
use crate::members::Members;

use crate::plugin::languages::Languages;
use crate::tunnel::Tunnel;

/// 等待异步事件的上限。
const WAIT: Duration = Duration::from_secs(15);

/// 生成一张自签叶证书，当作配对时互换的那张（真实 DER，指纹现算）。
fn certificate(name: &str) -> PeerCertificate {
    let key = rcgen::KeyPair::generate().unwrap();
    let params = rcgen::CertificateParams::new(vec![name.to_owned()]).unwrap();
    PeerCertificate::from_der(params.self_signed(&key).unwrap().der().to_vec()).unwrap()
}

/// 一台记录上钉着的证书，按钉的顺序。
fn pins(peers: &Peers, server_name: &str) -> Vec<PeerCertificate> {
    peers.lock().peers[server_name]
        .peer_pin
        .iter()
        .map(|pin| pin.certificate.clone())
        .collect()
}

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
            None,
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

    /// 接上本机证书：配对成立后会发给对方。
    fn set_certificate(&self, certificate: PeerCertificate) {
        self.peers
            .set_own_certificate(Arc::new(move || Some(certificate.clone())));
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
            Arc::new(Members::open(self.dir.path().join("members.json")).unwrap()),
            Arc::new(|| Languages::from_header(None)),
            endpoint,
            0,
            Arc::new(crate::relay_settings::RelaySettings::for_table(
                RelayTable::builtin_n0(),
            )),
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
    pair_named(a, b, "a.example.internal", "b.example.internal").await;
}

/// 同 [`pair`]，服务器名由调用方给（私用名的测试用）。
async fn pair_named(a: &TestCore, b: &TestCore, a_name: &str, b_name: &str) {
    let code = a.peers.issue_code();
    b.scan_code(&code, a).await.unwrap();
    a.request_now(b_name);
    a.approve(b_name).await.unwrap();
    a.wait_paired(b_name).await;
    b.wait_paired(a_name).await;
    a.wait_paired_names(b_name).await;
    b.wait_paired_names(a_name).await;
}

/// 配对成立时记成待钉、窗口从当前核心版本起算；证书一到就钉，回执后只留最新一张，删配对时 pin 一起没。
#[tokio::test]
async fn pins_start_pending_and_follow_certificates() {
    let a = TestCore::start("Alex", "a.example.internal").await;
    let mut b = TestCore::start("Alice", "b.example.internal").await;
    pair(&a, &b).await;

    let pending = PinState::Pending {
        since_version: crate::build_info::version().to_owned(),
    };
    for core in [&a, &b] {
        let inner = core.peers.lock();
        let record = inner.peers.values().next().unwrap();
        assert_eq!(record.pin_state, pending);
        assert!(record.peer_pin.is_empty());
    }

    let first = certificate("a.example.internal");
    let second = certificate("a.example.internal");
    b.peers.on_certificate(a.id_endpoint(), &first, false);
    b.peers.on_certificate(a.id_endpoint(), &second, false);
    assert_eq!(
        pins(&b.peers, "a.example.internal"),
        vec![first.clone(), second.clone()]
    );
    assert_eq!(
        b.peers.lock().peers["a.example.internal"].pin_state,
        PinState::Pinned
    );

    // 对方回执了我们最新的那张：旧的丢掉。
    b.peers.on_certificate(a.id_endpoint(), &second, true);
    assert_eq!(pins(&b.peers, "a.example.internal"), vec![second.clone()]);

    // 不是已配对的 endpoint 发来的证书不落盘。
    b.peers.on_certificate(
        SecretKey::generate().public(),
        &certificate("other.example.internal"),
        false,
    );
    assert_eq!(pins(&b.peers, "a.example.internal"), vec![second.clone()]);

    // 钉住的证书落盘，重启后还在。
    let (reopened, _) = b.reopen().await;
    assert_eq!(pins(&reopened, "a.example.internal"), vec![second.clone()]);

    // 删配对：记录连着 pin 一起删，文件里也不留。
    reopened.remove("a.example.internal").await.unwrap();
    assert!(!reopened.lock().peers.contains_key("a.example.internal"));
    assert!(
        !std::fs::read_to_string(b.store_path())
            .unwrap()
            .contains("spki_sha256")
    );
}

/// 老版本写的记录没有 pin 相关字段：读成待钉、没有入口，窗口起点补上当前核心版本并落盘。
#[tokio::test]
async fn old_records_read_as_pending_on_the_current_version() {
    let mut a = TestCore::start("Alex", "a.example.internal").await;
    let record = legacy_record(&SecretKey::generate().public().to_string());
    std::fs::write(
        a.store_path(),
        serde_json::json!({"peers": {"old.example.internal": record}}).to_string(),
    )
    .unwrap();

    let (reopened, _) = a.reopen().await;
    let inner = reopened.lock();
    let record = &inner.peers["old.example.internal"];
    assert_eq!(
        record.pin_state,
        PinState::Pending {
            since_version: crate::build_info::version().to_owned(),
        }
    );
    assert!(record.peer_pin.is_empty());
    drop(inner);

    let stored: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(a.store_path()).unwrap()).unwrap();
    assert_eq!(
        stored["peers"]["old.example.internal"]["pin_state"]["pending"]["since_version"],
        crate::build_info::version()
    );
}

/// 老版本写的记录形状（没有 peer_pin、pin_state）。
fn legacy_record(endpoint_id: &str) -> serde_json::Value {
    serde_json::json!({
        "server_name": "old.example.internal",
        "endpoint_id": endpoint_id,
        "relays": [],
        "addrs": [],
        "owner_user_id": "@alex:old.example.internal",
        "owner_display_name": "Alex",
        "paired_at_ms": 1,
        "phase": "paired",
    })
}

/// 两边都接了本机证书：配对成立后自动互换、钉住、回执，各留对方一张，状态转成已钉。
#[tokio::test]
async fn pairing_exchanges_certificates() {
    let a = TestCore::start("Alex", "a.example.internal").await;
    let b = TestCore::start("Alice", "b.example.internal").await;
    let a_certificate = certificate("a.example.internal");
    let b_certificate = certificate("b.example.internal");
    a.set_certificate(a_certificate.clone());
    b.set_certificate(b_certificate.clone());
    pair(&a, &b).await;

    a.wait_for(
        |core| !pins(&core.peers, "b.example.internal").is_empty(),
        "对方证书送到",
    )
    .await;
    b.wait_for(
        |core| !pins(&core.peers, "a.example.internal").is_empty(),
        "对方证书送到",
    )
    .await;
    assert_eq!(pins(&a.peers, "b.example.internal"), vec![b_certificate]);
    assert_eq!(pins(&b.peers, "a.example.internal"), vec![a_certificate]);
    assert_eq!(
        a.peers.lock().peers["b.example.internal"].pin_state,
        PinState::Pinned
    );
    assert_eq!(
        b.peers.lock().peers["a.example.internal"].pin_state,
        PinState::Pinned
    );
}

/// 本机证书接上得晚（配对先成立、安装产物后到）：已经有、还没 pin 的配对现在补发一次。
#[tokio::test]
async fn attaching_the_certificate_late_sends_it_to_an_unpinned_pairing() {
    let a = TestCore::start("Alex", "a.example.internal").await;
    let b = TestCore::start("Alice", "b.example.internal").await;
    pair(&a, &b).await;
    assert!(pins(&a.peers, "b.example.internal").is_empty());

    let b_certificate = certificate("b.example.internal");
    b.set_certificate(b_certificate.clone());
    assert!(b.peers.own_certificate().is_some());

    a.wait_for(
        |core| !pins(&core.peers, "b.example.internal").is_empty(),
        "补发的证书",
    )
    .await;
    assert_eq!(pins(&a.peers, "b.example.internal"), vec![b_certificate]);
}

/// 探活/连上成功时再送一次：配对后那几次没送到（对方离线）也补得上，不靠运气。
#[tokio::test]
async fn a_connected_probe_resends_a_certificate_that_never_arrived() {
    let a = TestCore::start("Alex", "a.example.internal").await;
    let b = TestCore::start("Alice", "b.example.internal").await;
    let b_certificate = certificate("b.example.internal");
    b.set_certificate(b_certificate.clone());
    pair(&a, &b).await;
    a.wait_for(
        |core| !pins(&core.peers, "b.example.internal").is_empty(),
        "第一次送达",
    )
    .await;
    // 模拟对方从没收到：清掉 a 手里的 pin，转回待钉。
    {
        let mut inner = a.peers.lock();
        let record = inner.peers.get_mut("b.example.internal").unwrap();
        record.peer_pin.clear();
        record.pin_state = PinState::Pending {
            since_version: "0.8.0".to_owned(),
        };
    }

    let (states, rx) = watch::channel(BTreeMap::new());
    b.peers.attach_forwarder(rx);
    states.send_replace(BTreeMap::from([(
        "a.example.internal".to_owned(),
        TunnelPeerStatus::Connected,
    )]));
    a.wait_for(
        |core| !pins(&core.peers, "b.example.internal").is_empty(),
        "连上后补发",
    )
    .await;
    assert_eq!(pins(&a.peers, "b.example.internal"), vec![b_certificate]);
}

/// 首次握手里看到的证书按升级专用 TOFU 钉住：只对还没有 pin 的已配对记录，钉过就不覆盖。
#[tokio::test]
async fn the_first_handshake_certificate_pins_only_an_unpinned_pairing() {
    let a = TestCore::start("Alex", "a.example.internal").await;
    let b = TestCore::start("Alice", "b.example.internal").await;
    pair(&a, &b).await;

    let first = certificate("a.example.internal");
    assert!(b.peers.tofu_pin("a.example.internal", &first));
    assert_eq!(pins(&b.peers, "a.example.internal"), vec![first]);
    assert_eq!(
        b.peers.lock().peers["a.example.internal"].pin_state,
        PinState::Pinned
    );

    let second = certificate("a.example.internal");
    assert!(!b.peers.tofu_pin("a.example.internal", &second));
    assert_eq!(pins(&b.peers, "a.example.internal").len(), 1);
}

/// 窗口已到期、还没 pin 的已配对记录如实报出来；没到期、已钉住的都不报。
#[tokio::test]
async fn expired_windows_are_reported_and_pinned_ones_are_not() {
    let a = TestCore::start("Alex", "a.example.internal").await;
    let b = TestCore::start("Alice", "b.example.internal").await;
    pair(&a, &b).await;
    {
        let mut inner = a.peers.lock();
        let record = inner.peers.get_mut("b.example.internal").unwrap();
        record.pin_state = PinState::Pending {
            since_version: "0.8.0".to_owned(),
        };
    }
    assert!(a.peers.expired_pending_on("0.9.0").is_empty(), "还没到期");
    assert_eq!(a.peers.expired_pending_on("0.10.0"), ["b.example.internal"]);

    a.peers
        .lock()
        .peers
        .get_mut("b.example.internal")
        .unwrap()
        .pin_state = PinState::Pinned;
    assert!(
        a.peers.expired_pending_on("0.10.0").is_empty(),
        "钉住就不报"
    );
}

/// 复审补测：`tls_verification` 要把落下的 pin 读出来交给隧道；否则生产里恒 `None`、永远走待钉。
#[tokio::test]
async fn review_tls_verification_returns_the_pinned_certificate() {
    use wonton_tunnel::federation::PeerVerification;

    let a = TestCore::start("Alex", "a.example.internal").await;
    let b = TestCore::start("Alice", "b.example.internal").await;
    pair(&a, &b).await;
    let cert = certificate("a.example.internal");
    b.peers.on_certificate(a.id_endpoint(), &cert, false);

    match b.peers.tls_verification("a.example.internal") {
        Some(PeerVerification::Pinned(pins)) => {
            assert_eq!(pins.len(), 1);
            assert_eq!(pins[0].spki_sha256(), cert.spki_sha256_bytes().unwrap());
        }
        other => panic!("有 pin 时应回 Pinned，实际 {other:?}"),
    }
    assert!(
        b.peers
            .tls_verification("unknown.example.internal")
            .is_none(),
        "没配对的查不到"
    );
}

/// 还没 pin 的已配对记录：`tls_verification` 回带窗口信息的待钉，并如实给出窗口起点（开发构建不算到期）。
#[tokio::test]
async fn tls_verification_reports_the_pending_window() {
    use wonton_tunnel::federation::PeerVerification;

    let a = TestCore::start("Alex", "a.example.internal").await;
    let b = TestCore::start("Alice", "b.example.internal").await;
    pair(&a, &b).await;
    b.peers
        .lock()
        .peers
        .get_mut("a.example.internal")
        .unwrap()
        .pin_state = PinState::Pending {
        since_version: "0.8.0".to_owned(),
    };

    match b.peers.tls_verification("a.example.internal") {
        Some(PeerVerification::Pending {
            since_version,
            expired,
        }) => {
            assert_eq!(since_version.as_deref(), Some("0.8.0"));
            assert!(!expired, "开发构建的版本不可排序，按没到期");
        }
        other => panic!("没 pin 时应回待钉，实际 {other:?}"),
    }
}

/// 钉住状态但 pin 数据坏了：fail-closed 成空根库，不回一条宽一点的待钉。
#[tokio::test]
async fn tls_verification_fails_closed_on_a_broken_pin() {
    use wonton_tunnel::federation::PeerVerification;

    let a = TestCore::start("Alex", "a.example.internal").await;
    let b = TestCore::start("Alice", "b.example.internal").await;
    pair(&a, &b).await;
    {
        let mut inner = b.peers.lock();
        let record = inner.peers.get_mut("a.example.internal").unwrap();
        record.pin_state = PinState::Pinned;
        record.peer_pin = vec![super::PeerPin {
            certificate: PeerCertificate {
                der: "YmFk".to_owned(),
                spki_sha256: "00".repeat(32),
            },
            pinned_at_ms: 1,
        }];
    }
    match b.peers.tls_verification("a.example.internal") {
        Some(PeerVerification::Pinned(pins)) => assert!(pins.is_empty(), "坏 pin 清空根库"),
        other => panic!("钉住的记录应回 Pinned，实际 {other:?}"),
    }
}
