//! 联邦出站的 TLS 终结（ADR-0112）：对本机 Tuwunel 当 TLS 服务端（出示本机 CA 签的证书），对对方当
//! TLS 客户端（根库只放钉住的叶，并单独比 SPKI）。入站的对应一半在另一张卡里；这里的对端用一个
//! 接受 `Forward` 流之后自己做 TLS 服务端的假核心模拟。

use std::{
    collections::BTreeSet,
    io,
    net::{Ipv4Addr, SocketAddr},
    pin::Pin,
    sync::{Arc, Mutex},
    task::{Context, Poll},
    time::Duration,
};

use iroh::{
    Endpoint, EndpointAddr, EndpointId, SecretKey,
    endpoint::{RecvStream, SendStream, presets},
};
use rcgen::{
    BasicConstraints, CertificateParams, DistinguishedName, DnType, ExtendedKeyUsagePurpose, IsCa,
    Issuer, KeyPair, KeyUsagePurpose,
};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, ReadBuf},
    net::TcpStream,
    sync::watch,
    task::JoinHandle,
    time::timeout,
};
use tokio_rustls::{
    TlsAcceptor, TlsConnector,
    rustls::{
        ClientConfig, RootCertStore, ServerConfig,
        crypto::aws_lc_rs::default_provider,
        pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName},
    },
};
use wonton_tunnel::{
    federation::{
        FEDERATION_ALPN, FederationForwarder, ForwarderTls, PeerStates, PeerStatus,
        PeerVerification, PinSeen, PinnedPeerCert, ServerLookup,
    },
    off_workers,
    wire::{Request, Status},
};

use crate::support::{Timeouts, WAIT, loopback_endpoint};

/// 探活放得很长：TLS 失败写进去的状态不用和探活抢。
const PING_INTERVAL: Duration = Duration::from_secs(60);
const PING_TIMEOUT: Duration = Duration::from_secs(5);

const B_NAME: &str = "b.example.internal";
const C_NAME: &str = "c.example.internal";
const PAYLOAD: &[u8] = b"federated bytes";
const REPLY: &[u8] = b"tuwunel reply";

type LocalCertificates = Arc<dyn Fn(&str) -> anyhow::Result<TlsAcceptor> + Send + Sync>;
type PeerPins = Arc<dyn Fn(&str) -> Option<PeerVerification> + Send + Sync>;

thread_local! {
    static TEST_CA: Arc<TestCa> = Arc::new(TestCa::new());
}

/// 测试的本机 CA：给“本机 Tuwunel 看到的”证书签名。
struct TestCa {
    cert: CertificateDer<'static>,
    issuer: Issuer<'static, KeyPair>,
}

impl TestCa {
    fn new() -> Self {
        let key = KeyPair::generate().unwrap();
        let mut params = CertificateParams::default();
        let mut name = DistinguishedName::new();
        name.push(DnType::CommonName, "tunnel federation test CA");
        params.distinguished_name = name;
        params.is_ca = IsCa::Ca(BasicConstraints::Constrained(0));
        params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
        let cert = params.self_signed(&key).unwrap();
        Self {
            cert: cert.der().clone(),
            issuer: Issuer::new(params, key),
        }
    }

    /// 给一个服务器名签一张服务端证书，链里带上 CA。
    fn acceptor(&self, name: &str) -> anyhow::Result<TlsAcceptor> {
        let key = KeyPair::generate()?;
        let leaf = leaf_params(name)?.signed_by(&key, &self.issuer)?;
        let key = PrivatePkcs8KeyDer::from(key.serialize_der()).into();
        server_acceptor(vec![leaf.der().clone(), self.cert.clone()], key)
    }

    fn local_certificates(self: &Arc<Self>) -> LocalCertificates {
        let ca = self.clone();
        Arc::new(move |name| ca.acceptor(name))
    }

    fn roots(&self) -> RootCertStore {
        let mut roots = RootCertStore::empty();
        roots.add(self.cert.clone()).unwrap();
        roots
    }
}

/// 对端的自签叶；它的公钥指纹就是测试里的 pin。
fn cert_for(name: &str) -> Result<(CertificateDer<'static>, PrivateKeyDer<'static>), rcgen::Error> {
    let key = KeyPair::generate()?;
    let leaf = leaf_params(name)?.self_signed(&key)?;
    Ok((
        leaf.der().clone(),
        PrivatePkcs8KeyDer::from(key.serialize_der()).into(),
    ))
}

fn leaf_params(name: &str) -> Result<CertificateParams, rcgen::Error> {
    let mut params = CertificateParams::new(vec![name.to_owned()])?;
    params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
    Ok(params)
}

fn server_acceptor(
    chain: Vec<CertificateDer<'static>>,
    key: PrivateKeyDer<'static>,
) -> anyhow::Result<TlsAcceptor> {
    let config = ServerConfig::builder_with_provider(Arc::new(default_provider()))
        .with_safe_default_protocol_versions()?
        .with_no_client_auth()
        .with_single_cert(chain, key)?;
    Ok(TlsAcceptor::from(Arc::new(config)))
}

/// 对端（假核心）：接受 `Forward` 流后回状态，接着当 TLS 服务端出示自己的自签叶，记录收到的字节并回
/// 固定回应。
struct TlsPeer {
    _endpoint: Endpoint,
    id: EndpointId,
    addr: EndpointAddr,
    recorded: Arc<Mutex<Vec<u8>>>,
    task: JoinHandle<()>,
}

impl TlsPeer {
    async fn start(acceptor: TlsAcceptor) -> Self {
        let endpoint = peer_endpoint().await;
        let addr = endpoint.addr();
        let recorded = Arc::new(Mutex::new(Vec::new()));
        let task = tokio::spawn(accept_loop(endpoint.clone(), acceptor, recorded.clone()));
        Self {
            id: addr.id,
            addr,
            _endpoint: endpoint,
            recorded,
            task,
        }
    }

    fn recorded(&self) -> Vec<u8> {
        self.recorded.lock().unwrap().clone()
    }
}

/// 只收联邦 ALPN 的 endpoint：没设 ALPN 的 endpoint 不接受任何入站连接。
async fn peer_endpoint() -> Endpoint {
    let builder = Endpoint::builder(presets::Minimal)
        .alpns(vec![FEDERATION_ALPN.to_vec()])
        .secret_key(SecretKey::generate())
        .clear_ip_transports()
        .bind_addr(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
        .unwrap();
    off_workers(move || builder.bind()).await.unwrap().unwrap()
}

impl Drop for TlsPeer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn accept_loop(endpoint: Endpoint, acceptor: TlsAcceptor, recorded: Arc<Mutex<Vec<u8>>>) {
    while let Some(incoming) = endpoint.accept().await {
        let Ok(mut accepting) = incoming.accept() else {
            continue;
        };
        let Ok(alpn) = accepting.alpn().await else {
            continue;
        };
        if alpn != FEDERATION_ALPN {
            continue;
        }
        let Ok(conn) = accepting.await else {
            continue;
        };
        let acceptor = acceptor.clone();
        let recorded = recorded.clone();
        tokio::spawn(async move {
            while let Ok((send, recv)) = conn.accept_bi().await {
                let acceptor = acceptor.clone();
                let recorded = recorded.clone();
                tokio::spawn(serve_stream(send, recv, acceptor, recorded));
            }
        });
    }
}

async fn serve_stream(
    mut send: SendStream,
    mut recv: RecvStream,
    acceptor: TlsAcceptor,
    recorded: Arc<Mutex<Vec<u8>>>,
) {
    let mut kind = [0u8; 1];
    if recv.read_exact(&mut kind).await.is_err() {
        return;
    }
    match Request::from_byte(kind[0]) {
        Some(Request::Ping) => {
            let _ = send.write_all(&[Status::Ok as u8]).await;
            send.finish().ok();
        }
        Some(Request::Forward) => {
            if send.write_all(&[Status::Ok as u8]).await.is_err() {
                return;
            }
            let io = StreamIo { send, recv };
            let Ok(mut tls) = acceptor.accept(io).await else {
                return;
            };
            let mut body = Vec::new();
            if timeout(WAIT, tls.read_to_end(&mut body)).await.is_err() {
                return;
            }
            recorded.lock().unwrap().extend_from_slice(&body);
            let _ = tls.write_all(REPLY).await;
            let _ = tls.shutdown().await;
        }
        _ => {
            let _ = send.write_all(&[Status::UnsupportedRequest as u8]).await;
            send.finish().ok();
        }
    }
}

/// 隧道流当 `AsyncRead`/`AsyncWrite` 用，交给 TLS。
struct StreamIo {
    send: SendStream,
    recv: RecvStream,
}

impl AsyncRead for StreamIo {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        AsyncRead::poll_read(Pin::new(&mut self.recv), cx, buf)
    }
}

impl AsyncWrite for StreamIo {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        AsyncWrite::poll_write(Pin::new(&mut self.send), cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        AsyncWrite::poll_flush(Pin::new(&mut self.send), cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        AsyncWrite::poll_shutdown(Pin::new(&mut self.send), cx)
    }
}

/// 本机转发器：查找表里只有 B。`pins` 决定这台服务器的校验严格度。
async fn start_forwarder(peer: &TlsPeer, pins: PeerPins) -> FederationForwarder {
    start_forwarder_with(peer, pins, None).await
}

/// 同 [`start_forwarder`]，另给一个待钉握手成功时的 TOFU 回调。
async fn start_forwarder_with(
    peer: &TlsPeer,
    pins: PeerPins,
    pin_seen: Option<PinSeen>,
) -> FederationForwarder {
    let lookup: ServerLookup = {
        let (id, addr) = (peer.id, peer.addr.clone());
        Arc::new(move |name: &str| (name == B_NAME).then(|| (id, addr.clone())))
    };
    let tls = ForwarderTls {
        local_certificates: TEST_CA.with(|ca| ca.local_certificates()),
        peer_pin: pins,
        pin_seen,
    };
    let (_paired, paired) = watch::channel(BTreeSet::from([B_NAME.to_owned()]));
    FederationForwarder::bind(
        loopback_endpoint(SecretKey::generate(), 0, Timeouts::Short).await,
        SocketAddr::from((Ipv4Addr::LOCALHOST, 0)),
        lookup,
        paired,
        PING_INTERVAL,
        PING_TIMEOUT,
        Some(tls),
    )
    .await
    .unwrap()
}

fn pinned_to(cert: &CertificateDer<'static>) -> PeerPins {
    let pin = PinnedPeerCert::from_der(cert.clone()).unwrap();
    let verification = PeerVerification::Pinned(vec![pin]);
    Arc::new(move |name: &str| (name == B_NAME).then(|| verification.clone()))
}

fn pending() -> PeerPins {
    Arc::new(|_: &str| {
        Some(PeerVerification::Pending {
            since_version: Some("0.8.0".to_owned()),
            expired: false,
        })
    })
}

/// 记录查不到：出站按 `unwrap_or` 的待钉缺省走。
fn unknown() -> PeerPins {
    Arc::new(|_: &str| None)
}

/// Tuwunel 经代理发起 CONNECT，读到回应头；之后的字节属于 TLS。
async fn connect(listen: SocketAddr, name: &str) -> (TcpStream, u16) {
    let mut tcp = TcpStream::connect(listen).await.unwrap();
    tcp.write_all(format!("CONNECT {name}:8448 HTTP/1.1\r\nHost: {name}:8448\r\n\r\n").as_bytes())
        .await
        .unwrap();
    let status = read_status(&mut tcp).await;
    (tcp, status)
}

/// 一个字节一个字节读回应头，不多读。
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
    String::from_utf8(head)
        .unwrap()
        .split(' ')
        .nth(1)
        .and_then(|code| code.parse().ok())
        .expect("回应头里有状态码")
}

async fn closed_after(mut tcp: TcpStream) {
    let mut rest = Vec::new();
    timeout(WAIT, tcp.read_to_end(&mut rest))
        .await
        .expect("等待连接关闭超时")
        .unwrap();
    assert!(rest.is_empty(), "错误回应之后还有 {rest:?}");
}

/// Tuwunel 对核心做 TLS 客户端：只信测试 CA。
async fn tuwunel_tls(
    ca: &TestCa,
    name: &str,
    tcp: TcpStream,
) -> tokio_rustls::client::TlsStream<TcpStream> {
    let config = ClientConfig::builder_with_provider(Arc::new(default_provider()))
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_root_certificates(ca.roots())
        .with_no_client_auth();
    TlsConnector::from(Arc::new(config))
        .connect(ServerName::try_from(name.to_owned()).unwrap(), tcp)
        .await
        .unwrap()
}

async fn wait_offline(forwarder: &FederationForwarder, name: &str) -> PeerStatus {
    let mut states: watch::Receiver<PeerStates> = forwarder.watch_status();
    timeout(
        WAIT,
        states.wait_for(|states| matches!(states.get(name), Some(PeerStatus::Offline { .. }))),
    )
    .await
    .expect("等待离线状态超时")
    .unwrap()
    .get(name)
    .unwrap()
    .clone()
}

/// 两端握手都过：本机 Tuwunel 验 CA 签的证书，核心验对端的钉证书，字节原样转发。
#[tokio::test]
async fn terminates_tls_when_the_pin_matches() {
    let ca = TEST_CA.with(|ca| ca.clone());
    let (leaf, key) = cert_for(B_NAME).unwrap();
    let peer = TlsPeer::start(server_acceptor(vec![leaf.clone()], key).unwrap()).await;
    let forwarder = start_forwarder(&peer, pinned_to(&leaf)).await;

    let (tcp, status) = connect(forwarder.listen_addr(), B_NAME).await;
    assert_eq!(status, 200);
    let mut tls = tuwunel_tls(&ca, B_NAME, tcp).await;
    tls.write_all(PAYLOAD).await.unwrap();
    tls.shutdown().await.unwrap();
    let mut reply = Vec::new();
    timeout(WAIT, tls.read_to_end(&mut reply))
        .await
        .expect("等待回应超时")
        .unwrap();
    assert_eq!(reply, REPLY);
    assert_eq!(peer.recorded(), PAYLOAD);
    assert_eq!(forwarder.outbound_errors().tls_failed, 0);
}

/// 对端出示的是另一把钥匙的证书：根库里有钉的叶，但 SPKI 对不上，回 502 并记离线。
#[tokio::test]
async fn rejects_a_certificate_with_the_wrong_pin() {
    let (served, key) = cert_for(B_NAME).unwrap();
    let (pinned, _) = cert_for(B_NAME).unwrap();
    let peer = TlsPeer::start(server_acceptor(vec![served], key).unwrap()).await;
    let forwarder = start_forwarder(&peer, pinned_to(&pinned)).await;

    let (tcp, status) = connect(forwarder.listen_addr(), B_NAME).await;
    assert_eq!(status, 502);
    closed_after(tcp).await;
    let status = wait_offline(&forwarder, B_NAME).await;
    let PeerStatus::Offline { last_error, .. } = status else {
        panic!("不是离线状态：{status:?}");
    };
    assert_eq!(last_error.code(), "peer-certificate");
    assert_eq!(forwarder.outbound_errors().tls_failed, 1);
}

/// 证书就是钉的那张，但名字不是 CONNECT 的目标：照样失败。
#[tokio::test]
async fn rejects_a_certificate_for_another_name() {
    let (leaf, key) = cert_for(C_NAME).unwrap();
    let peer = TlsPeer::start(server_acceptor(vec![leaf.clone()], key).unwrap()).await;
    let forwarder = start_forwarder(&peer, pinned_to(&leaf)).await;

    let (tcp, status) = connect(forwarder.listen_addr(), B_NAME).await;
    assert_eq!(status, 502);
    closed_after(tcp).await;
    let status = wait_offline(&forwarder, B_NAME).await;
    let PeerStatus::Offline { last_error, .. } = status else {
        panic!("不是离线状态：{status:?}");
    };
    assert_eq!(last_error.code(), "peer-certificate");
}

/// 旧记录没有 pin：只校名字，自签证书照常放行。
#[tokio::test]
async fn accepts_any_certificate_for_the_name_when_pending() {
    let ca = TEST_CA.with(|ca| ca.clone());
    let (leaf, key) = cert_for(B_NAME).unwrap();
    let peer = TlsPeer::start(server_acceptor(vec![leaf], key).unwrap()).await;
    let forwarder = start_forwarder(&peer, pending()).await;

    let (tcp, status) = connect(forwarder.listen_addr(), B_NAME).await;
    assert_eq!(status, 200);
    let mut tls = tuwunel_tls(&ca, B_NAME, tcp).await;
    tls.write_all(PAYLOAD).await.unwrap();
    tls.shutdown().await.unwrap();
    let mut reply = Vec::new();
    timeout(WAIT, tls.read_to_end(&mut reply))
        .await
        .expect("等待回应超时")
        .unwrap();
    assert_eq!(reply, REPLY);
    assert_eq!(peer.recorded(), PAYLOAD);
    assert_eq!(forwarder.outbound_errors().tls_failed, 0);
}

/// 待钉时名字仍然要校：证书是 C 的，CONNECT 的是 B，失败。记录查不到也按待钉缺省走。
#[tokio::test]
async fn rejects_another_name_when_pending() {
    let (leaf, key) = cert_for(C_NAME).unwrap();
    let peer = TlsPeer::start(server_acceptor(vec![leaf], key).unwrap()).await;
    let forwarder = start_forwarder(&peer, unknown()).await;
    let (tcp, status) = connect(forwarder.listen_addr(), B_NAME).await;
    assert_eq!(status, 502);
    closed_after(tcp).await;
}

/// 一张 2000–2001 年的过期叶，链和名字都对。
fn expired_leaf(
    name: &str,
) -> Result<(CertificateDer<'static>, PrivateKeyDer<'static>), rcgen::Error> {
    let key = KeyPair::generate()?;
    let mut params = leaf_params(name)?;
    params.not_before = rcgen::date_time_ymd(2000, 1, 1);
    params.not_after = rcgen::date_time_ymd(2001, 1, 1);
    let leaf = params.self_signed(&key)?;
    Ok((
        leaf.der().clone(),
        PrivatePkcs8KeyDer::from(key.serialize_der()).into(),
    ))
}

/// 同 [`expired_leaf`]，但叶由测试 CA 签、链里带 CA：根库钉 CA 时链能过，只剩有效期这一关。
fn expired_leaf_signed_by_ca(ca: &TestCa, name: &str) -> TlsAcceptor {
    let key = KeyPair::generate().unwrap();
    let mut params = leaf_params(name).unwrap();
    params.not_before = rcgen::date_time_ymd(2000, 1, 1);
    params.not_after = rcgen::date_time_ymd(2001, 1, 1);
    let leaf = params.signed_by(&key, &ca.issuer).unwrap();
    server_acceptor(
        vec![leaf.der().clone(), ca.cert.clone()],
        PrivatePkcs8KeyDer::from(key.serialize_der()).into(),
    )
    .unwrap()
}

/// 钉的是 CA 时链能过、叶的 SPKI 却不同：证明根库之外确实单独比了 SPKI。
#[tokio::test]
async fn rejects_a_chain_that_validates_but_has_another_spki() {
    let ca = TEST_CA.with(|ca| ca.clone());
    let key = KeyPair::generate().unwrap();
    let leaf = leaf_params(B_NAME)
        .unwrap()
        .signed_by(&key, &ca.issuer)
        .unwrap();
    let acceptor = server_acceptor(
        vec![leaf.der().clone(), ca.cert.clone()],
        PrivatePkcs8KeyDer::from(key.serialize_der()).into(),
    )
    .unwrap();
    let peer = TlsPeer::start(acceptor).await;
    let forwarder = start_forwarder(&peer, pinned_to(&ca.cert.clone())).await;

    let (tcp, status) = connect(forwarder.listen_addr(), B_NAME).await;
    assert_eq!(status, 502);
    closed_after(tcp).await;
    let PeerStatus::Offline { last_error, .. } = wait_offline(&forwarder, B_NAME).await else {
        panic!("不是离线状态");
    };
    assert_eq!(last_error.code(), "peer-certificate");
    assert!(
        format!("{last_error}").contains("does not match the pinned key"),
        "{last_error}"
    );
}

/// 钉住的链上叶过期：握手失败并报“过期”，不放行。
#[tokio::test]
async fn rejects_an_expired_peer_certificate_under_a_pin() {
    let ca = TEST_CA.with(|ca| ca.clone());
    let peer = TlsPeer::start(expired_leaf_signed_by_ca(&ca, B_NAME)).await;
    let forwarder = start_forwarder(&peer, pinned_to(&ca.cert.clone())).await;

    let (tcp, status) = connect(forwarder.listen_addr(), B_NAME).await;
    assert_eq!(status, 502);
    closed_after(tcp).await;
    let PeerStatus::Offline { last_error, .. } = wait_offline(&forwarder, B_NAME).await else {
        panic!("不是离线状态");
    };
    assert!(format!("{last_error}").contains("expired"), "{last_error}");
}

/// 待钉也看有效期：过期叶不放行。
#[tokio::test]
async fn rejects_an_expired_peer_certificate_when_pending() {
    let (leaf, key) = expired_leaf(B_NAME).unwrap();
    let peer = TlsPeer::start(server_acceptor(vec![leaf], key).unwrap()).await;
    let forwarder = start_forwarder(&peer, pending()).await;

    let (tcp, status) = connect(forwarder.listen_addr(), B_NAME).await;
    assert_eq!(status, 502);
    closed_after(tcp).await;
    let PeerStatus::Offline { last_error, .. } = wait_offline(&forwarder, B_NAME).await else {
        panic!("不是离线状态");
    };
    assert!(format!("{last_error}").contains("expired"), "{last_error}");
}

/// 待钉窗口已经到期：仍然放行（如实报“对端还没升级”），不直接断。
#[tokio::test]
async fn accepts_by_name_when_the_pending_window_expired() {
    let ca = TEST_CA.with(|ca| ca.clone());
    let (leaf, key) = cert_for(B_NAME).unwrap();
    let peer = TlsPeer::start(server_acceptor(vec![leaf], key).unwrap()).await;
    let expired: PeerPins = Arc::new(|_: &str| {
        Some(PeerVerification::Pending {
            since_version: Some("0.8.0".to_owned()),
            expired: true,
        })
    });
    let forwarder = start_forwarder(&peer, expired).await;

    let (tcp, status) = connect(forwarder.listen_addr(), B_NAME).await;
    assert_eq!(status, 200);
    let mut tls = tuwunel_tls(&ca, B_NAME, tcp).await;
    tls.write_all(PAYLOAD).await.unwrap();
    tls.shutdown().await.unwrap();
}

/// `Pinned` 是空列表（pin 记录坏了被清空）：根库空，fail-closed 回 502。
#[tokio::test]
async fn rejects_when_the_pin_list_is_empty() {
    let (leaf, key) = cert_for(B_NAME).unwrap();
    let peer = TlsPeer::start(server_acceptor(vec![leaf], key).unwrap()).await;
    let empty: PeerPins = Arc::new(|_: &str| Some(PeerVerification::Pinned(Vec::new())));
    let forwarder = start_forwarder(&peer, empty).await;

    let (tcp, status) = connect(forwarder.listen_addr(), B_NAME).await;
    assert_eq!(status, 502);
    closed_after(tcp).await;
    assert_eq!(forwarder.outbound_errors().tls_failed, 1);
}

/// 待钉握手成功时把对方的叶交给 TOFU 回调（核心据此钉住，不用等配对通道送证书）。
#[tokio::test]
async fn a_pending_handshake_hands_the_peer_leaf_to_the_tofu_sink() {
    let ca = TEST_CA.with(|ca| ca.clone());
    let (leaf, key) = cert_for(B_NAME).unwrap();
    let leaf_der = leaf.clone();
    let peer = TlsPeer::start(server_acceptor(vec![leaf], key).unwrap()).await;
    let seen = Arc::new(Mutex::new(Vec::new()));
    let sink: PinSeen = {
        let seen = seen.clone();
        Arc::new(move |name: &str, der: Vec<u8>| {
            seen.lock().unwrap().push((name.to_owned(), der));
        })
    };
    let forwarder = start_forwarder_with(&peer, pending(), Some(sink)).await;

    let (tcp, status) = connect(forwarder.listen_addr(), B_NAME).await;
    assert_eq!(status, 200);
    let mut tls = tuwunel_tls(&ca, B_NAME, tcp).await;
    tls.write_all(PAYLOAD).await.unwrap();
    tls.shutdown().await.unwrap();
    timeout(WAIT, async {
        while seen.lock().unwrap().is_empty() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("等待 TOFU 回调超时");
    let seen = seen.lock().unwrap().clone();
    assert_eq!(seen[0].0, B_NAME);
    assert_eq!(seen[0].1, leaf_der.as_ref());
}
