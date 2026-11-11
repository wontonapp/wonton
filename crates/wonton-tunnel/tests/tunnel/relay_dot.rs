//! 配对码里带 FQDN 尾点的中继地址也要能配上（#200）。
//!
//! 现有的中继测试都让客户端跳过 TLS 校验（`insecure_skip_verify`）；这里起一台自建 CA 签出真证书链的
//! 测试中继（`relay.test`），两端只信这把 CA，iroh 对中继的 TLS 校验（rustls + webpki）真实跑一遍。
//! 码里的中继地址故意写成带尾点的形式（和 #200 里真实核心发出来的一样）：解析后规范成不带的
//! [`canonical_url`](wonton_tunnel::relays::canonical_url) 一种写法，配对经中继走通。

use std::{
    net::{Ipv4Addr, SocketAddr},
    str::FromStr,
    sync::{Arc, Mutex},
};

use iroh::endpoint::presets;
use iroh::protocol::Router;
use iroh::tls::CaTlsConfig;
use iroh::{
    Endpoint, RelayMode, SecretKey,
    dns::{DnsResolver, NameserverConfig},
};
use iroh_relay::server::{
    CertConfig, RelayConfig as RelayServerConfig, Server as RelayServer, ServerConfig, TlsConfig,
};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use tokio::{task::JoinHandle, time::timeout};
use url::Url;
use wonton_tunnel::pair::{
    PAIR_ALPN, PairHandler, PairProtocol, PairResponse, PairingCode, PairingSecret, RejectReason,
    SECRET_LEN, pair,
};
use wonton_tunnel::relays::{Relay, relay_map};

use crate::support::{WAIT, init_tracing};

/// 只认一个密钥，记下谁来配过。
#[derive(Debug)]
struct OneCode {
    secret: PairingSecret,
    paired: Mutex<Vec<iroh::EndpointId>>,
}

impl PairHandler for OneCode {
    fn pair(
        &self,
        device: iroh::EndpointId,
        secret: &PairingSecret,
        _device_name: &str,
    ) -> PairResponse {
        if *secret != self.secret {
            return PairResponse::Rejected {
                reason: RejectReason::UnknownCode,
                message: "没发过这个码".to_owned(),
            };
        }
        self.paired.lock().unwrap().push(device);
        PairResponse::Paired
    }
}

/// 客户端只信测试 CA，叶子证书的 SAN 不带尾点。
fn ca_chain() -> (
    CertificateDer<'static>,
    Vec<CertificateDer<'static>>,
    PrivateKeyDer<'static>,
) {
    let ca_key = rcgen::KeyPair::generate().unwrap();
    let mut ca_params = rcgen::CertificateParams::default();
    ca_params
        .distinguished_name
        .push(rcgen::DnType::CommonName, "relay-dot test CA");
    ca_params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
    ca_params.key_usages = vec![rcgen::KeyUsagePurpose::KeyCertSign];
    let ca_cert = ca_params.self_signed(&ca_key).unwrap();

    let leaf_key = rcgen::KeyPair::generate().unwrap();
    let leaf_params = rcgen::CertificateParams::new(vec!["relay.test".to_owned()]).unwrap();
    let issuer = rcgen::Issuer::new(ca_params, &ca_key);
    let leaf = leaf_params.signed_by(&leaf_key, &issuer).unwrap();
    (
        ca_cert.der().clone(),
        vec![leaf.der().clone(), ca_cert.der().clone()],
        PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(leaf_key.serialize_der())),
    )
}

/// 极小 UDP DNS：`relay.test` 的 A 查询回 127.0.0.1，其余类型回 NOERROR 空答案。
/// 带不带尾点的写法在线上是同一个查询，域名解析不出测试的差别；差别只在 URL 的拼法。
async fn spawn_dns() -> (SocketAddr, JoinHandle<()>) {
    use tokio::net::UdpSocket;
    let socket = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let addr = socket.local_addr().unwrap();
    let task = tokio::spawn(async move {
        let mut buf = [0u8; 512];
        loop {
            let Ok((len, peer)) = socket.recv_from(&mut buf).await else {
                return;
            };
            let query = &buf[..len];
            // 跳过 qname：从第 12 字节起数标签，到 0 结束，跟 qtype/qclass。
            let Some((qtype, end)) = question(query) else {
                continue;
            };
            let mut reply = Vec::with_capacity(64);
            reply.extend_from_slice(&query[0..2]); // 事务 ID
            reply.extend_from_slice(&[0x81, 0x80, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]);
            reply.extend_from_slice(&query[12..end]);
            if qtype == 1 {
                // A 记录：relay.test → 127.0.0.1。
                reply[7] = 0x01; // ancount = 1
                reply.extend_from_slice(&[0xc0, 0x0c, 0x00, 0x01, 0x00, 0x01]);
                reply.extend_from_slice(&[0x00, 0x00, 0x00, 0x3c, 0x00, 0x04, 127, 0, 0, 1]);
            }
            let _ = socket.send_to(&reply, peer).await;
        }
    });
    (addr, task)
}

/// 问题段里取 qtype：返回 `(qtype, 问题段结束的偏移)`。
fn question(query: &[u8]) -> Option<(u16, usize)> {
    if query.len() < 17 {
        return None;
    }
    let mut i = 12;
    while i < query.len() && query[i] != 0 {
        i += query[i] as usize + 1;
    }
    i += 1;
    (i + 4 <= query.len()).then(|| (u16::from_be_bytes([query[i], query[i + 1]]), i + 4))
}

async fn spawn_relay(
    chain: Vec<CertificateDer<'static>>,
    key: PrivateKeyDer<'static>,
) -> RelayServer {
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let server_config = rustls::ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth();
    let server_config = server_config.with_single_cert(chain, key).unwrap();
    let tls = TlsConfig::new(
        (Ipv4Addr::LOCALHOST, 0),
        CertConfig::Manual { server_config },
    );
    let mut relay = RelayServerConfig::new((Ipv4Addr::LOCALHOST, 0));
    relay.tls = Some(tls);
    let mut config = ServerConfig::default();
    config.relay = Some(relay);
    RelayServer::spawn(config).await.unwrap()
}

/// 只连给定中继的 endpoint：不做 IP 直连（中继是唯一路径），只信测试的 CA，域名走测试的 DNS。
async fn relay_only_endpoint(
    key: SecretKey,
    relays: Vec<Relay>,
    ca: CertificateDer<'static>,
    dns: &DnsResolver,
) -> Endpoint {
    Endpoint::builder(presets::Minimal)
        .secret_key(key)
        .clear_ip_transports()
        .dns_resolver(dns.clone())
        .relay_mode(RelayMode::Custom(relay_map(&relays, usize::MAX)))
        .ca_tls_config(CaTlsConfig::custom_roots([ca]))
        .bind()
        .await
        .unwrap()
}

async fn wait_relay_connected(endpoint: &Endpoint) {
    timeout(WAIT, endpoint.online())
        .await
        .expect("中继应完成 TLS 握手并登记 endpoint");
}

#[tokio::test]
async fn a_code_with_a_dotted_relay_url_pairs_over_a_ca_verified_relay() {
    init_tracing();
    let (ca, chain, key) = ca_chain();
    let relay = spawn_relay(chain, key).await;
    let port = relay.https_addr().unwrap().port();
    let (dns_addr, dns_task) = spawn_dns().await;
    let dns = DnsResolver::builder()
        .add_nameserver_config(NameserverConfig::udp(dns_addr.ip()).with_port(dns_addr.port()))
        .disable_fallback()
        .build();

    // 服务端的中继表用我们的写法（不带尾点），像真实核心从配置读出来的一样。
    let server_url = format!("https://relay.test:{port}/");
    let handler = Arc::new(OneCode {
        secret: PairingSecret::from_bytes([7; SECRET_LEN]),
        paired: Mutex::new(Vec::new()),
    });
    let server_endpoint = relay_only_endpoint(
        SecretKey::generate(),
        vec![Relay {
            url: server_url.parse().unwrap(),
            token: None,
            tier: 0,
        }],
        ca.clone(),
        &dns,
    )
    .await;
    let router = Router::builder(server_endpoint.clone())
        .accept(PAIR_ALPN, PairProtocol::new(handler.clone()))
        .spawn();
    wait_relay_connected(&server_endpoint).await;

    // 码里故意写带尾点的地址：#200 里真实核心发出来的拼法。
    let dotted = format!("https://relay.test.:{port}/");
    let code = PairingCode {
        server_id: server_endpoint.id(),
        relays: vec![Relay {
            url: dotted.parse().unwrap(),
            token: None,
            tier: 0,
        }],
        direct_addrs: Vec::new(),
        base_domain: "wonton.localhost".to_owned(),
        core_url: Url::from_str("http://wonton.localhost:18480/").unwrap(),
        secret: handler.secret.clone(),
    };
    // 解析码：中继地址应规范成不带的唯一写法。
    let parsed = PairingCode::parse(&code.to_uri()).unwrap();
    assert_eq!(
        parsed.relays[0].url.as_str(),
        server_url,
        "带尾点的码要规范成一种写法"
    );

    let phone = relay_only_endpoint(SecretKey::generate(), parsed.relays.clone(), ca, &dns).await;
    wait_relay_connected(&phone).await;

    let reply = timeout(WAIT, pair(&phone, &parsed, "phone"))
        .await
        .expect("配对应在期限内完成")
        .unwrap();
    assert_eq!(reply, PairResponse::Paired);
    assert_eq!(
        *handler.paired.lock().unwrap(),
        vec![phone.id()],
        "登记的应是握手认证过的设备身份"
    );
    router.shutdown().await.unwrap();
    phone.close().await;
    server_endpoint.close().await;
    relay.shutdown().await.unwrap();
    dns_task.abort();
}
