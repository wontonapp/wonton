use super::*;
use iroh::{
    Endpoint,
    endpoint::{Connection, presets},
    protocol::Router,
};
use std::{net::Ipv4Addr, sync::Arc, time::Duration};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use wonton_protocol::{
    MatrixOpenIdToken, ProxySettings, SecretString, SharedPlugin, SharedPlugins, TabState,
};
use wonton_tunnel::sharing::{
    AuthenticatedShares, BoxFuture, ProxyHub, SHARE_ALPN, ShareClient, ShareHandler, ShareLimits,
    ShareProtocol, ShareRefusal, ShareSession, ShareStream,
};

struct Handler;

fn shares() -> SharedPlugins {
    SharedPlugins {
        user_id: "@receiver:identity.test".into(),
        plugins: vec![SharedPlugin {
            plugin_id: "app.same".into(),
            title: "Shared".into(),
            web_url: "http://app-same.source.test/".into(),
            state: TabState::Ready,
        }],
    }
}

impl ShareHandler for Handler {
    fn authenticate(
        &self,
        _: MatrixOpenIdToken,
    ) -> BoxFuture<'_, Result<AuthenticatedShares, ShareRefusal>> {
        Box::pin(async {
            Ok(AuthenticatedShares {
                shares: shares(),
                generation: 1,
            })
        })
    }
    fn list<'a>(
        &'a self,
        _: &'a ShareSession,
    ) -> BoxFuture<'a, Result<SharedPlugins, ShareRefusal>> {
        Box::pin(async { Ok(shares()) })
    }
    fn forward<'a>(
        &'a self,
        _: &'a ShareSession,
        _: Connection,
        mut stream: ShareStream,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            let mut request = Vec::new();
            let mut byte = [0];
            while !request.ends_with(b"\r\n\r\n") && request.len() < 8192 {
                if stream.read_exact(&mut byte).await.is_err() {
                    return;
                }
                request.push(byte[0]);
            }
            let _ = stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                .await;
            let _ = stream.shutdown().await;
        })
    }
}

async fn endpoint() -> Endpoint {
    Endpoint::builder(presets::Minimal)
        .clear_ip_transports()
        .bind_addr((Ipv4Addr::LOCALHOST, 0))
        .unwrap()
        .bind()
        .await
        .unwrap()
}

async fn source(owner: &str, router: &Router, hub: &ProxyHub) -> Source {
    let client = ShareClient::connect(
        endpoint().await,
        router.endpoint().addr(),
        MatrixOpenIdToken {
            access_token: SecretString::new("test-openid"),
            matrix_server_name: "identity.test".into(),
            expires_in: 3600,
        },
    )
    .await
    .unwrap();
    let friend = wonton_matrix::FriendSource {
        user_id: owner.try_into().unwrap(),
        display_name: None,
    };
    let mut source = Source::new(&friend);
    source.attach(client, hub).await.unwrap();
    source
}

fn settings(source: &Source) -> ProxySettings {
    let SharedAppsState::Ready { proxy } = source.snapshot().state else {
        panic!("source not ready")
    };
    proxy
}

async fn status(proxy: &ProxySettings) -> u16 {
    reqwest::Client::builder()
        .no_proxy()
        .proxy(reqwest::Proxy::all(crate::chat::proxy_url(proxy)).unwrap())
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap()
        .head("http://app-same.source.test/")
        .send()
        .await
        .unwrap()
        .status()
        .as_u16()
}

#[tokio::test]
async fn closing_one_source_revokes_its_hub_route_and_keeps_the_other_source_ready() {
    let hub = ProxyHub::bind().await.unwrap();
    let router = Router::builder(endpoint().await)
        .accept(
            SHARE_ALPN,
            ShareProtocol::new(Arc::new(Handler), ShareLimits::default()),
        )
        .spawn();
    let view = watch::Sender::new(PluginsViewModel::default());
    let mut observed = view.subscribe();
    let alice = source("@alice:a.test", &router, &hub).await;
    let bob = source("@bob:b.test", &router, &hub).await;
    let (a, b) = (settings(&alice), settings(&bob));
    assert_eq!((a.host.as_str(), a.port), (b.host.as_str(), b.port));
    // 用户名固定，来源之间靠随机密码区分。
    assert!(a.password != b.password);
    assert_eq!(status(&a).await, 200);
    assert_eq!(status(&b).await, 200);
    let closed = alice.client().unwrap();
    alice.watch_closed(view.clone());
    bob.watch_closed(view.clone());
    let sources = HashMap::from([("@alice:a.test".into(), alice), ("@bob:b.test".into(), bob)]);
    publish(&view, &sources);
    closed.close();
    tokio::time::timeout(
        Duration::from_secs(5),
        observed.wait_for(|view| {
            view.shared_apps.as_ref().is_some_and(|shared| {
                matches!(shared.sources[0].state, SharedAppsState::Failed { .. })
            })
        }),
    )
    .await
    .unwrap()
    .unwrap();
    publish(&view, &sources);
    let snapshot = view.borrow().shared_apps.clone().unwrap();
    assert_eq!(snapshot.sources[0].apps.len(), 1);
    assert!(matches!(
        snapshot.sources[0].state,
        SharedAppsState::Failed { .. }
    ));
    assert!(matches!(
        snapshot.sources[1].state,
        SharedAppsState::Ready { .. }
    ));
    assert_eq!(status(&a).await, 407);
    assert_eq!(status(&b).await, 200);
    drop(sources);
    assert_eq!(status(&b).await, 407);
    router.shutdown().await.unwrap();
}
