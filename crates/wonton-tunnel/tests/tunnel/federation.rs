//! 联邦转发的行为测试：两台“服务器”都在本机进程内，各自一个 endpoint、入站协议和出站转发器；
//! Tuwunel 用本机 TCP 连接模拟，上游是回环上的记录/回显服务。不依赖外网和 Docker。

use std::{
    collections::{BTreeMap, BTreeSet},
    net::{Ipv4Addr, SocketAddr},
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use iroh::{
    EndpointAddr, EndpointId, SecretKey,
    endpoint::{Connection, ConnectionError, VarInt},
    protocol::DynProtocolHandler,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::watch,
    task::JoinHandle,
    time::timeout,
};
use wonton_tunnel::{
    Allowlist, TunnelServer,
    federation::{
        FEDERATION_ALPN, FederationForwarder, FederationProtocol, OutboundErrors, PeerStates,
        PeerStatus, ServerLookup,
    },
    pair::{PairHandler, PairProtocol, PairResponse, PairingSecret, RejectReason},
};

use crate::support::{Timeouts, WAIT, loopback_endpoint};

use wonton_tunnel::ALPN as TUNNEL_ALPN;

/// 对探活周期和超时取测试用的小值：状态变化都在几秒内出现。
const PING_INTERVAL: Duration = Duration::from_millis(300);
const PING_TIMEOUT: Duration = Duration::from_millis(2000);

/// 上游回给每条连接的固定字节。转发器只转发字节，内容是什么都行。
const REPLY: &[u8] = b"tuwunel-reply";

/// 测试用的 Matrix 服务器名。绑定由测试的查找表定，和真实命名没有关系。
const B_NAME: &str = "b.example.internal";
const C_NAME: &str = "c.example.internal";

/// 所有测试共用的“配对记录”：服务器名 → 对方 endpoint ID 和地址。出站的查找函数查它。
type Pairings = Arc<Mutex<BTreeMap<String, (EndpointId, EndpointAddr)>>>;

fn pairings() -> Pairings {
    Arc::new(Mutex::new(BTreeMap::new()))
}

/// 按 [`Pairings`] 实现的查找函数，和核心按 `core/peers.json` 实现的形状一样。
fn lookup_from(table: &Pairings) -> ServerLookup {
    let table = table.clone();
    Arc::new(move |name| table.lock().unwrap().get(name).cloned())
}

/// 出码方拒绝一切的配对处理方：联邦测试不开配对。
#[derive(Debug)]
struct NoPairing;

impl PairHandler for NoPairing {
    fn pair(&self, _: EndpointId, _: &PairingSecret, _: &str) -> PairResponse {
        PairResponse::Rejected {
            reason: RejectReason::Closed,
            message: "联邦测试不开配对".into(),
        }
    }
}

/// 测试里当 Tuwunel 用的上游：把每条连接收到的字节记下来，回一段固定回应，然后关连接。
struct Upstream {
    addr: SocketAddr,
    received: Arc<Mutex<Vec<u8>>>,
    connections: Arc<AtomicUsize>,
    task: JoinHandle<()>,
}

impl Upstream {
    async fn spawn() -> Self {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let addr = listener.local_addr().unwrap();
        let received = Arc::new(Mutex::new(Vec::new()));
        let connections = Arc::new(AtomicUsize::new(0));
        let task = {
            let (received, connections) = (received.clone(), connections.clone());
            tokio::spawn(async move {
                while let Ok((mut tcp, _)) = listener.accept().await {
                    connections.fetch_add(1, Ordering::SeqCst);
                    let received = received.clone();
                    tokio::spawn(async move {
                        let mut buf = Vec::new();
                        if timeout(WAIT, tcp.read_to_end(&mut buf)).await.is_err() {
                            panic!("上游等 EOF 超时");
                        }
                        received.lock().unwrap().extend_from_slice(&buf);
                        let _ = tcp.write_all(REPLY).await;
                        let _ = tcp.shutdown().await;
                    });
                }
            })
        };
        Self {
            addr,
            received,
            connections,
            task,
        }
    }

    /// 上游到目前为止收到的全部字节。
    fn received(&self) -> Vec<u8> {
        self.received.lock().unwrap().clone()
    }

    fn connections(&self) -> usize {
        self.connections.load(Ordering::SeqCst)
    }
}

impl Drop for Upstream {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// 测试里的一台“服务器”：入站联邦协议 + 出站转发器 + 上游，可停掉后用同样的身份和端口重启。
struct TestPeer {
    key: SecretKey,
    id: EndpointId,
    addr: EndpointAddr,
    device_allowlist: Allowlist,
    server_allowlist: Allowlist,
    upstream: Upstream,
    /// endpoint 的 UDP 端口和出站转发器的监听地址，重启要用同一组。
    endpoint_port: u16,
    listen: SocketAddr,
    lookup: ServerLookup,
    server: Option<TunnelServer>,
    _listener: wonton_tunnel::TunnelListener,
    forwarder: Option<FederationForwarder>,
    /// 已配对的服务器名，转发器据此开始探活。
    paired: watch::Sender<BTreeSet<String>>,
    states: watch::Receiver<PeerStates>,
}

impl TestPeer {
    /// `lookup` 是本方出站用的“服务器名 → 对方”查找函数。
    async fn start(key: SecretKey, lookup: ServerLookup) -> Self {
        let endpoint = loopback_endpoint(key.clone(), 0, Timeouts::Short).await;
        let endpoint_port = endpoint.bound_sockets()[0].port();
        let addr = endpoint.addr();
        let device_allowlist = Allowlist::default();
        let server_allowlist = Allowlist::default();
        let upstream = Upstream::spawn().await;
        let federation = FederationProtocol::new(server_allowlist.clone(), upstream.addr);
        let (paired, paired_rx) = watch::channel(BTreeSet::new());
        let forwarder = FederationForwarder::bind(
            endpoint.clone(),
            SocketAddr::from((Ipv4Addr::LOCALHOST, 0)),
            lookup.clone(),
            paired_rx,
            PING_INTERVAL,
            PING_TIMEOUT,
        )
        .await
        .unwrap();
        let (server, listener) = TunnelServer::spawn_embedded_with_pairing_and(
            endpoint,
            device_allowlist.clone(),
            PairProtocol::new(Arc::new(NoPairing)),
            [(
                FEDERATION_ALPN,
                Box::new(federation) as Box<dyn DynProtocolHandler>,
            )],
        );
        let states = forwarder.watch_status();
        Self {
            key,
            id: addr.id,
            addr,
            device_allowlist,
            server_allowlist,
            upstream,
            endpoint_port,
            listen: forwarder.listen_addr(),
            lookup,
            server: Some(server),
            _listener: listener,
            forwarder: Some(forwarder),
            paired,
            states,
        }
    }

    fn forwarder(&self) -> &FederationForwarder {
        self.forwarder.as_ref().expect("转发器没在运行")
    }

    async fn stop(&mut self) {
        self.forwarder.take();
        self.server.take().expect("服务端没在运行").shutdown().await;
    }

    /// 用同样的身份、端口、名单和查找函数重启。停掉之前要先 [`stop`](Self::stop)。
    async fn restart(&mut self) {
        assert!(self.server.is_none(), "服务端还在运行");
        wait_until_port_is_free(self.endpoint_port).await;
        let endpoint =
            loopback_endpoint(self.key.clone(), self.endpoint_port, Timeouts::Short).await;
        assert_eq!(endpoint.addr().id, self.id, "重启后 endpoint ID 变了");
        let federation = FederationProtocol::new(self.server_allowlist.clone(), self.upstream.addr);
        let forwarder = FederationForwarder::bind(
            endpoint.clone(),
            self.listen,
            self.lookup.clone(),
            self.paired.subscribe(),
            PING_INTERVAL,
            PING_TIMEOUT,
        )
        .await
        .unwrap();
        let (server, listener) = TunnelServer::spawn_embedded_with_pairing_and(
            endpoint,
            self.device_allowlist.clone(),
            PairProtocol::new(Arc::new(NoPairing)),
            [(
                FEDERATION_ALPN,
                Box::new(federation) as Box<dyn DynProtocolHandler>,
            )],
        );
        self.states = forwarder.watch_status();
        self.server = Some(server);
        self._listener = listener;
        self.forwarder = Some(forwarder);
    }

    /// 改配对名单。
    fn set_paired<const N: usize>(&self, names: [&str; N]) {
        self.paired
            .send_replace(names.iter().map(|name| (*name).to_owned()).collect());
    }

    /// 等某台对方服务器从状态表里消失。
    async fn wait_gone(&self, name: &str) {
        timeout(WAIT, async {
            loop {
                if !self.states.borrow().contains_key(name) {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("等状态条目消失超时");
    }

    /// 等某台对方服务器的状态变成想要的值。
    async fn wait_for(&self, name: &str, want: fn(&PeerStatus) -> bool) {
        let mut states = self.states.clone();
        timeout(
            WAIT,
            states.wait_for(|states| states.get(name).is_some_and(want)),
        )
        .await
        .expect("等待状态变化超时")
        .unwrap();
    }
}

/// 等旧 endpoint 的 UDP 端口空出来（同 support 里的做法）。
async fn wait_until_port_is_free(port: u16) {
    let deadline = tokio::time::Instant::now() + WAIT;
    while std::net::UdpSocket::bind((Ipv4Addr::LOCALHOST, port)).is_err() {
        assert!(
            tokio::time::Instant::now() < deadline,
            "旧 endpoint 的 UDP 端口 {port} 一直没有释放"
        );
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
}

/// 经本方转发器 CONNECT 到 `name` 的联邦端口，确认回 200 后发 `payload`，半关写端，读回经隧道
/// 回来的全部字节。
async fn federate(listen: SocketAddr, name: &str, payload: &[u8]) -> Vec<u8> {
    federate_on_port(listen, name, 8448, payload).await
}

async fn federate_on_port(listen: SocketAddr, name: &str, port: u16, payload: &[u8]) -> Vec<u8> {
    let (mut tcp, status) = connect(listen, &format!("{name}:{port}")).await;
    assert_eq!(status, 200, "CONNECT {name}:{port} 没有成功");
    tcp.write_all(payload).await.unwrap();
    tcp.shutdown().await.unwrap();
    let mut reply = Vec::new();
    timeout(WAIT, tcp.read_to_end(&mut reply))
        .await
        .expect("等待回应超时")
        .unwrap();
    reply
}

/// 发一个 CONNECT（Tuwunel 经代理出站时发的形状），读到回应头结束为止，返回连接和状态码。
async fn connect(listen: SocketAddr, target: &str) -> (TcpStream, u16) {
    let mut tcp = TcpStream::connect(listen).await.unwrap();
    tcp.write_all(format!("CONNECT {target} HTTP/1.1\r\nHost: {target}\r\n\r\n").as_bytes())
        .await
        .unwrap();
    let status = read_status(&mut tcp).await;
    (tcp, status)
}

/// 一个字节一个字节读回应头，不多读：之后的字节是隧道里的内容。
async fn read_status(tcp: &mut TcpStream) -> u16 {
    let mut head = Vec::new();
    while !head.ends_with(b"\r\n\r\n") {
        let mut byte = [0u8; 1];
        timeout(WAIT, tcp.read_exact(&mut byte))
            .await
            .expect("等待回应头超时")
            .expect("回应头没读完连接就关了");
        head.push(byte[0]);
    }
    let line = String::from_utf8(head).unwrap();
    line.split(' ')
        .nth(1)
        .and_then(|code| code.parse().ok())
        .expect("回应头里有状态码")
}

/// 回了错误状态之后连接被关，后面再没有字节。
async fn closed_after(mut tcp: TcpStream) {
    let mut rest = Vec::new();
    timeout(WAIT, tcp.read_to_end(&mut rest))
        .await
        .expect("等待连接关闭超时")
        .unwrap();
    assert!(rest.is_empty(), "错误回应之后还有 {rest:?}");
}

/// 断言这条连接被对方用 403 关闭（未授权：对端这次连的 ALPN 的名单里没有它）。
async fn assert_closed_unauthorized(conn: Connection) {
    let reason = timeout(WAIT, conn.closed())
        .await
        .expect("等待连接关闭超时");
    let ConnectionError::ApplicationClosed(close) = reason else {
        panic!("不是应用层关闭：{reason:?}");
    };
    assert_eq!(close.error_code, VarInt::from_u32(403));
}

/// 把两台服务器互相放进对方的名单和查找表，完成一次 A → B 的联邦往返。
async fn pair_a_to_b(a: &TestPeer, b: &TestPeer, table: &Pairings, payload: &[u8]) {
    table
        .lock()
        .unwrap()
        .insert(B_NAME.to_owned(), (b.id, b.addr.clone()));
    a.server_allowlist.allow(b.id);
    b.server_allowlist.allow(a.id);
    let reply = federate(a.listen, B_NAME, payload).await;
    assert_eq!(b.upstream.received(), payload);
    assert_eq!(reply, REPLY);
    a.wait_for(B_NAME, |status| *status == PeerStatus::Connected)
        .await;
}

/// 两台互在名单里：A 的本机 Tuwunel CONNECT 到 B 的联邦端口后发任意字节，B 的上游原样收到，
/// 回的字节 A 也原样收到。
#[tokio::test]
async fn forwards_a_connect_to_the_paired_server() {
    let table = pairings();
    let a = TestPeer::start(SecretKey::generate(), lookup_from(&table)).await;
    let b = TestPeer::start(SecretKey::generate(), lookup_from(&table)).await;
    pair_a_to_b(&a, &b, &table, &[0xde, 0xad, 0xbe, 0xef]).await;
    // 共享连接：再来一条连接走同一条 iroh 连接，B 的上游各自收到自己的字节。
    let reply = federate(a.listen, B_NAME, b"second").await;
    assert_eq!(
        b.upstream.received(),
        [&[0xde, 0xad, 0xbe, 0xef][..], b"second"].concat()
    );
    assert_eq!(reply, REPLY);
}

#[tokio::test]
async fn forwards_named_server_discovery_on_port_443() {
    let table = pairings();
    let a = TestPeer::start(SecretKey::generate(), lookup_from(&table)).await;
    let b = TestPeer::start(SecretKey::generate(), lookup_from(&table)).await;
    pair_quietly(&a, &b, &table);

    let payload = b"GET /.well-known/matrix/server HTTP/1.1\r\n\r\n";
    let reply = federate_on_port(a.listen, B_NAME, 443, payload).await;
    assert_eq!(b.upstream.received(), payload);
    assert_eq!(b.upstream.connections(), 1);
    assert_eq!(reply, REPLY);
    assert_eq!(a.forwarder().outbound_errors(), OutboundErrors::default());
}

#[tokio::test]
async fn refuses_private_server_discovery_but_forwards_its_federation_port() {
    let table = pairings();
    let a = TestPeer::start(SecretKey::generate(), lookup_from(&table)).await;
    let b = TestPeer::start(SecretKey::generate(), lookup_from(&table)).await;
    let private_name = "b.wonton.internal";
    table
        .lock()
        .unwrap()
        .insert(private_name.to_owned(), (b.id, b.addr.clone()));
    a.server_allowlist.allow(b.id);
    b.server_allowlist.allow(a.id);

    let (tcp, status) = connect(a.listen, &format!("{private_name}:443")).await;
    assert_eq!(status, 502);
    closed_after(tcp).await;
    assert_eq!(a.forwarder().outbound_errors(), OutboundErrors::default());
    assert!(a.states.borrow().is_empty());
    assert_eq!(b.upstream.connections(), 0);

    let reply = federate(a.listen, private_name, b"private-federation").await;
    assert_eq!(reply, REPLY);
    assert_eq!(b.upstream.received(), b"private-federation");
}

#[tokio::test]
async fn refuses_unknown_server_discovery_without_using_another_pairing() {
    let table = pairings();
    let a = TestPeer::start(SecretKey::generate(), lookup_from(&table)).await;
    let b = TestPeer::start(SecretKey::generate(), lookup_from(&table)).await;
    pair_quietly(&a, &b, &table);

    let (tcp, status) = connect(a.listen, "stranger.example.internal:443").await;
    assert_eq!(status, 403);
    closed_after(tcp).await;
    assert_eq!(a.forwarder().outbound_errors().unknown_server, 1);
    assert_eq!(a.forwarder().outbound_errors().invalid_request, 0);
    assert!(a.states.borrow().is_empty());
    assert_eq!(b.upstream.connections(), 0);
}

/// 服务器名不认识：回 403 后关闭，转发器记下原因，状态表里没有这个服务器。
#[tokio::test]
async fn refuses_connects_to_unknown_server_names() {
    let table = pairings();
    let a = TestPeer::start(SecretKey::generate(), lookup_from(&table)).await;
    let (tcp, status) = connect(a.listen, "stranger.example.internal:8448").await;
    assert_eq!(status, 403);
    closed_after(tcp).await;
    assert_eq!(a.forwarder().outbound_errors().unknown_server, 1);
    assert!(a.states.borrow().is_empty());
}

/// 不支持的端口回 502；不计入出站错误，也不碰对方。
#[tokio::test]
async fn answers_bad_gateway_for_other_ports() {
    let table = pairings();
    let a = TestPeer::start(SecretKey::generate(), lookup_from(&table)).await;
    let b = TestPeer::start(SecretKey::generate(), lookup_from(&table)).await;
    table
        .lock()
        .unwrap()
        .insert(B_NAME.to_owned(), (b.id, b.addr.clone()));
    a.server_allowlist.allow(b.id);
    b.server_allowlist.allow(a.id);
    let (tcp, status) = connect(a.listen, &format!("{B_NAME}:9448")).await;
    assert_eq!(status, 502);
    closed_after(tcp).await;
    assert_eq!(a.forwarder().outbound_errors(), OutboundErrors::default());
    assert_eq!(b.upstream.connections(), 0);
}

/// 连接开头不是 CONNECT（这里是一段 TLS 记录）：回 400 后关闭并计数。
#[tokio::test]
async fn refuses_connections_without_a_connect_request() {
    let table = pairings();
    let a = TestPeer::start(SecretKey::generate(), lookup_from(&table)).await;
    for opening in [
        &b"\x16\x03\x01\x00\x02\x01\x00\r\n\r\n"[..],
        b"GET / HTTP/1.1\r\nHost: b.example.internal\r\n\r\n",
        b"CONNECT b.example.internal HTTP/1.1\r\n\r\n",
    ] {
        let mut tcp = TcpStream::connect(a.listen).await.unwrap();
        tcp.write_all(opening).await.unwrap();
        assert_eq!(read_status(&mut tcp).await, 400, "{opening:?}");
        closed_after(tcp).await;
    }
    assert_eq!(a.forwarder().outbound_errors().invalid_request, 3);
}

/// B 把 A 移出名单：A 的状态变成 RemovedByPeer。
#[tokio::test]
async fn reports_removed_by_peer_when_the_peer_revokes_us() {
    let table = pairings();
    let a = TestPeer::start(SecretKey::generate(), lookup_from(&table)).await;
    let b = TestPeer::start(SecretKey::generate(), lookup_from(&table)).await;
    pair_a_to_b(&a, &b, &table, b"before").await;
    b.server_allowlist.revoke(&a.id);
    a.wait_for(B_NAME, |status| *status == PeerStatus::RemovedByPeer)
        .await;
}

/// B 停掉：A 变 Offline；B 回来后 A 恢复 Connected。
#[tokio::test]
async fn goes_offline_when_the_peer_stops_and_recovers_when_it_returns() {
    let table = pairings();
    let a = TestPeer::start(SecretKey::generate(), lookup_from(&table)).await;
    let mut b = TestPeer::start(SecretKey::generate(), lookup_from(&table)).await;
    pair_a_to_b(&a, &b, &table, b"before").await;
    b.stop().await;
    a.wait_for(B_NAME, |status| {
        matches!(status, PeerStatus::Offline { .. })
    })
    .await;
    b.restart().await;
    a.wait_for(B_NAME, |status| *status == PeerStatus::Connected)
        .await;
}

/// 两边互相放进名单、名字绑上查找表，但一个字节的联邦流量都不发。
fn pair_quietly(a: &TestPeer, b: &TestPeer, table: &Pairings) {
    table
        .lock()
        .unwrap()
        .insert(B_NAME.to_owned(), (b.id, b.addr.clone()));
    a.server_allowlist.allow(b.id);
    b.server_allowlist.allow(a.id);
}

/// 只配对、没有任何出站流量：转发器自己建客户端开始探活，一个周期内状态就是 Connected，
/// 探活不走对方的上游。
#[tokio::test]
async fn probes_a_paired_server_without_any_outbound_traffic() {
    let table = pairings();
    let a = TestPeer::start(SecretKey::generate(), lookup_from(&table)).await;
    let b = TestPeer::start(SecretKey::generate(), lookup_from(&table)).await;
    pair_quietly(&a, &b, &table);
    a.set_paired([B_NAME]);
    a.wait_for(B_NAME, |status| *status == PeerStatus::Connected)
        .await;
    assert_eq!(b.upstream.connections(), 0, "探活不该碰上游");
}

/// 配对后新加的服务器马上开始探活；对方不在（endpoint 在，但没有联邦协议）时状态就是 Offline。
#[tokio::test]
async fn probes_a_server_added_to_the_pairing_later() {
    let table = pairings();
    let a = TestPeer::start(SecretKey::generate(), lookup_from(&table)).await;
    // 只起了 endpoint、没挂联邦协议的空壳当“对方不在”。
    let ghost = loopback_endpoint(SecretKey::generate(), 0, Timeouts::Short).await;
    let ghost_addr = ghost.addr();
    table
        .lock()
        .unwrap()
        .insert(C_NAME.to_owned(), (ghost_addr.id, ghost_addr));
    a.set_paired([C_NAME]);
    a.wait_for(C_NAME, |status| {
        matches!(status, PeerStatus::Offline { .. })
    })
    .await;
}

/// 配对删掉：这台服务器从状态表里消失，探活停下。
#[tokio::test]
async fn stops_probing_a_server_removed_from_the_pairing() {
    let table = pairings();
    let a = TestPeer::start(SecretKey::generate(), lookup_from(&table)).await;
    let b = TestPeer::start(SecretKey::generate(), lookup_from(&table)).await;
    pair_quietly(&a, &b, &table);
    a.set_paired([B_NAME]);
    a.wait_for(B_NAME, |status| *status == PeerStatus::Connected)
        .await;
    a.set_paired([]);
    a.wait_gone(B_NAME).await;
}

/// 设备名单的 ID 连不上联邦 ALPN，服务器名单的 ID 连不上隧道 ALPN：都以 403 关闭。
#[tokio::test]
async fn keeps_device_and_server_allowlists_separate() {
    let table = pairings();
    let a = TestPeer::start(SecretKey::generate(), lookup_from(&table)).await;
    // 配对过的设备连联邦 ALPN：联邦的名单里没有它，403。
    let device = loopback_endpoint(SecretKey::generate(), 0, Timeouts::Short).await;
    a.device_allowlist.allow(device.id());
    let conn = device
        .connect(a.addr.clone(), FEDERATION_ALPN)
        .await
        .unwrap();
    assert_closed_unauthorized(conn).await;
    // 配对过的服务器连隧道 ALPN：设备的名单里没有它，403。
    let server = loopback_endpoint(SecretKey::generate(), 0, Timeouts::Short).await;
    a.server_allowlist.allow(server.id());
    let conn = server.connect(a.addr.clone(), TUNNEL_ALPN).await.unwrap();
    assert_closed_unauthorized(conn).await;
}

/// 一个服务器名只会被转到它绑定的 endpoint ID（ADR-0050“代价与风险”）：CONNECT C 的连接到 C，
/// B 一条连接都收不到；CONNECT B 的连接到 B。
#[tokio::test]
async fn routes_a_server_name_only_to_its_bound_endpoint() {
    let table = pairings();
    let a = TestPeer::start(SecretKey::generate(), lookup_from(&table)).await;
    let b = TestPeer::start(SecretKey::generate(), lookup_from(&table)).await;
    let c = TestPeer::start(SecretKey::generate(), lookup_from(&table)).await;
    table.lock().unwrap().extend([
        (B_NAME.to_owned(), (b.id, b.addr.clone())),
        (C_NAME.to_owned(), (c.id, c.addr.clone())),
    ]);
    a.server_allowlist.allow(b.id);
    a.server_allowlist.allow(c.id);
    b.server_allowlist.allow(a.id);
    c.server_allowlist.allow(a.id);

    let reply = federate(a.listen, C_NAME, b"to-c").await;
    assert_eq!(c.upstream.received(), b"to-c");
    assert_eq!(b.upstream.connections(), 0);
    assert_eq!(reply, REPLY);
    a.wait_for(C_NAME, |status| *status == PeerStatus::Connected)
        .await;

    let reply = federate(a.listen, B_NAME, b"to-b").await;
    assert_eq!(b.upstream.received(), b"to-b");
    assert_eq!(c.upstream.connections(), 1);
    assert_eq!(reply, REPLY);
    a.wait_for(B_NAME, |status| *status == PeerStatus::Connected)
        .await;
}
