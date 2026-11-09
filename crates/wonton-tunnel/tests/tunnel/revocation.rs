//! 中继令牌先有效、随后被撤销时的重连行为。

use std::collections::HashMap;
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use iroh::endpoint::{QuicTransportConfig, presets};
use iroh::test_utils::run_relay_server;
use iroh::{
    Endpoint, EndpointId, RelayMap, RelayMode, RelayUrl, SecretKey, Watcher, tls::CaTlsConfig,
};
use iroh_relay::server::{
    Access, AccessControl, CertConfig, ClientRequest, DynAccessControl,
    RelayConfig as RelayServerConfig, Server as RelayServer, ServerConfig, TlsConfig,
};
use tokio::net::{TcpListener, TcpStream};
use wonton_tunnel::relays::{RelayTable, relay_map, spawn_tier_fallback};

use crate::support::{WAIT, init_tracing};

/// iroh actor 的退避从 10 ms 起、带抖动并封顶 16 s；十秒窗口的上限按几十次取值。
const REVOKED_MAIN_LIMIT: usize = 40;
const REVOKED_PROBE_LIMIT: usize = 60;
const RECONNECT_WINDOW: Duration = Duration::from_secs(10);
const FALLBACK_AFTER: Duration = Duration::from_secs(2);

type AttemptCounts = Arc<Mutex<HashMap<EndpointId, Arc<AtomicUsize>>>>;

/// 接受令牌可在测试中切换；每次真实的 `/relay` 握手都按 endpoint 身份计数。
#[derive(Debug)]
struct RevocableToken {
    token: String,
    accepted: Arc<AtomicBool>,
    counts: AttemptCounts,
}

impl RevocableToken {
    fn new(token: &str, accepted: Arc<AtomicBool>, counts: AttemptCounts) -> Self {
        Self {
            token: token.to_owned(),
            accepted,
            counts,
        }
    }

    fn attempts(&self, endpoint: EndpointId) -> usize {
        self.counts
            .lock()
            .unwrap()
            .get(&endpoint)
            .map_or(0, |count| count.load(Ordering::Relaxed))
    }

    fn endpoint_ids(&self) -> Vec<EndpointId> {
        self.counts.lock().unwrap().keys().copied().collect()
    }
}

impl AccessControl for RevocableToken {
    async fn on_connect(&self, request: &ClientRequest) -> Access {
        let count = {
            let mut counts = self.counts.lock().unwrap();
            counts
                .entry(request.endpoint_id())
                .or_insert_with(|| Arc::new(AtomicUsize::new(0)))
                .clone()
        };
        count.fetch_add(1, Ordering::Relaxed);
        if self.accepted.load(Ordering::Relaxed)
            && request.auth_token().as_deref() == Some(self.token.as_str())
        {
            Access::Allow
        } else {
            Access::Deny { reason: None }
        }
    }
}

async fn spawn_relay(access: Arc<dyn DynAccessControl>) -> anyhow::Result<RelayServer> {
    let (_certs, server_config) = iroh_relay::server::testing::self_signed_tls_certs_and_config();
    let tls = TlsConfig::new(
        (Ipv4Addr::LOCALHOST, 0),
        CertConfig::Manual { server_config },
    );
    let mut relay = RelayServerConfig::new((Ipv4Addr::LOCALHOST, 0));
    relay.tls = Some(tls);
    relay.key_cache_capacity = Some(1024);
    relay.access = access;
    let mut config = ServerConfig::default();
    config.relay = Some(relay);
    Ok(RelayServer::spawn(config).await?)
}

/// 保持中继地址不变，只切换后端，模拟令牌仍在原地址但服务端已换准入状态。
struct Front {
    addr: SocketAddr,
    backend: tokio::sync::watch::Sender<Option<SocketAddr>>,
}

impl Front {
    async fn spawn() -> anyhow::Result<Self> {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await?;
        let addr = listener.local_addr()?;
        let (backend, mut backend_rx) = tokio::sync::watch::channel(None);
        tokio::spawn(async move {
            loop {
                let accepted = tokio::select! {
                    accepted = listener.accept() => accepted,
                    _ = backend_rx.changed() => continue,
                };
                let (mut downstream, _) = match accepted {
                    Ok(value) => value,
                    Err(_) => continue,
                };
                let Some(backend) = *backend_rx.borrow() else {
                    continue;
                };
                if let Ok(mut upstream) = TcpStream::connect(backend).await {
                    tokio::spawn(async move {
                        let _ = tokio::io::copy_bidirectional(&mut downstream, &mut upstream).await;
                    });
                }
            }
        });
        Ok(Self { addr, backend })
    }

    fn set_backend(&self, backend: Option<SocketAddr>) {
        self.backend.send_replace(backend);
    }

    fn relay_url(&self) -> RelayUrl {
        format!("https://{}", self.addr).parse().unwrap()
    }
}

/// 只走给定的本地中继，构造与生产恢复探针相同：Minimal、随机身份、无直连。
async fn relay_endpoint(key: SecretKey, relays: RelayMap) -> Endpoint {
    init_tracing();
    let transport = QuicTransportConfig::builder()
        .max_idle_timeout(Some(Duration::from_secs(3).try_into().unwrap()))
        .keep_alive_interval(Duration::from_secs(1))
        .build();
    Endpoint::builder(presets::Minimal)
        .secret_key(key)
        .clear_ip_transports()
        .relay_mode(RelayMode::Custom(relays))
        .ca_tls_config(CaTlsConfig::insecure_skip_verify())
        .transport_config(transport)
        .bind()
        .await
        .unwrap()
}

async fn wait_relay_connected(endpoint: &Endpoint, url: &RelayUrl) {
    tokio::time::timeout(WAIT, async {
        loop {
            if endpoint
                .home_relay_status()
                .get()
                .iter()
                .any(|status| status.url() == url && status.is_connected())
            {
                return;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .expect("等令牌有效的中继连上超时");
}

async fn wait_auth_denied(endpoint: &Endpoint, url: &RelayUrl) {
    tokio::time::timeout(WAIT, async {
        loop {
            if endpoint
                .home_relay_status()
                .get()
                .iter()
                .any(|status| status.url() == url && status.auth_denied_reason().is_some())
            {
                return;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .expect("令牌撤销后没有观察到 auth_denied");
}

async fn wait_new_endpoint(access: &RevocableToken, known: &[EndpointId]) -> EndpointId {
    tokio::time::timeout(WAIT, async {
        loop {
            if let Some(endpoint) = access
                .endpoint_ids()
                .into_iter()
                .find(|endpoint| !known.contains(endpoint))
            {
                return endpoint;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .expect("恢复探针没有完成令牌握手")
}

#[tokio::test]
async fn a_revoked_token_bounds_main_and_probe_reconnects() -> anyhow::Result<()> {
    let accepted = Arc::new(AtomicBool::new(true));
    let counts = Arc::new(Mutex::new(HashMap::new()));
    let access = Arc::new(RevocableToken::new("tok", accepted.clone(), counts));
    let front = Front::spawn().await?;
    let valid = spawn_relay(access.clone()).await?;
    front.set_backend(Some(valid.https_addr().expect("配置了 TLS 地址")));

    let (_second_map, second_url, second_relay) = run_relay_server().await?;
    let main_table = RelayTable::parse(&format!("https://tok@{}", front.addr)).unwrap();
    let fallback_table =
        RelayTable::parse(&format!("https://tok@{};{second_url}", front.addr)).unwrap();
    let main = relay_endpoint(SecretKey::generate(), relay_map(main_table.relays(), 1)).await;
    let fallback_endpoint =
        relay_endpoint(SecretKey::generate(), relay_map(fallback_table.relays(), 1)).await;
    let mut fallback = spawn_tier_fallback(
        fallback_endpoint.clone(),
        fallback_table,
        FALLBACK_AFTER,
        Some(CaTlsConfig::insecure_skip_verify()),
    );
    wait_relay_connected(&main, &front.relay_url()).await;
    wait_relay_connected(&fallback_endpoint, &front.relay_url()).await;

    let main_id = main.id();
    let fallback_id = fallback_endpoint.id();
    let main_start = access.attempts(main_id);
    let fallback_start = access.attempts(fallback_id);
    assert!(main_start >= 1, "主 endpoint 没有完成有效令牌握手");
    assert!(fallback_start >= 1, "分档 endpoint 没有完成有效令牌握手");

    // 两个 endpoint 已经成功过，再把同一前门切到仍运行但拒绝该令牌的真中继。
    accepted.store(false, Ordering::Relaxed);
    valid.shutdown().await?;
    let revoked = spawn_relay(access.clone()).await?;
    front.set_backend(Some(revoked.https_addr().expect("配置了 TLS 地址")));
    let deadline = Instant::now() + RECONNECT_WINDOW;

    let relay_url = front.relay_url();
    let ((), (), ()) = tokio::join!(
        wait_auth_denied(&main, &relay_url),
        wait_auth_denied(&fallback_endpoint, &relay_url),
        fallback.wait_enabled_tiers(2),
    );
    let probe_id = wait_new_endpoint(&access, &[main_id, fallback_id]).await;
    tokio::time::sleep(deadline.saturating_duration_since(Instant::now())).await;

    let main_attempts = access.attempts(main_id) - main_start;
    let fallback_attempts = access.attempts(fallback_id) - fallback_start;
    let probe_attempts = access.attempts(probe_id);
    eprintln!(
        "revoked relay attempts over {RECONNECT_WINDOW:?}: main={main_attempts} fallback={fallback_attempts} probe={probe_attempts}"
    );
    assert!(
        main_attempts >= 1,
        "令牌撤销后没有观察到主 endpoint 的拒绝握手"
    );
    assert!(
        fallback_attempts >= 1,
        "令牌撤销后没有观察到分档 endpoint 的拒绝握手"
    );
    assert!(
        probe_attempts >= 1,
        "令牌撤销后没有观察到恢复探针的拒绝握手"
    );
    assert!(
        main_attempts <= REVOKED_MAIN_LIMIT,
        "令牌撤销后主 endpoint 十秒连接数 {main_attempts} 超过 {REVOKED_MAIN_LIMIT}"
    );
    assert!(
        fallback_attempts <= REVOKED_MAIN_LIMIT,
        "令牌撤销后分档 endpoint 十秒连接数 {fallback_attempts} 超过 {REVOKED_MAIN_LIMIT}"
    );
    assert!(
        probe_attempts <= REVOKED_PROBE_LIMIT,
        "令牌撤销后恢复探针连接数 {probe_attempts} 超过 {REVOKED_PROBE_LIMIT}"
    );

    main.close().await;
    fallback_endpoint.close().await;
    revoked.shutdown().await?;
    second_relay.shutdown().await?;
    Ok(())
}
