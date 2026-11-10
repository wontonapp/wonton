use super::{SharePeer, Sharing, now_ms};
use crate::gateway::{
    Gateway, router,
    tests::{CORE, Harness, PLUGIN, Reply, TOKEN},
};
use crate::plugin::manifest::PluginId;
use crate::{friends::Friends, share_identity::IdentityVerifier, shares::Shares};
use axum::{body::Body, extract::Request};
use hyper::{Method, StatusCode};
use std::{sync::Arc, time::Duration};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tower::ServiceExt;
use wonton_protocol::{ShareGrant, ShareGrantUpdate, ShareRecipient};

#[path = "guard_tests.rs"]
mod guard_tests;
#[path = "proxy_tests.rs"]
mod proxy_tests;

struct Fixture {
    h: Harness,
    gateway: Arc<Gateway>,
    sharing: Arc<Sharing>,
    peer: SharePeer,
    clock: Arc<std::sync::atomic::AtomicU64>,
}
impl Fixture {
    async fn new() -> Self {
        let h = Harness::start(None).await;
        let friends = Arc::new(Friends::open(h.data_dir.join("test-friends.json")).unwrap());
        let friend = friends
            .accept("@alice:example.test", Some("Alice"), now_ms(), |_| false)
            .unwrap();
        let shares =
            Arc::new(Shares::open(h.data_dir.join("test-shares.json"), friends.clone()).unwrap());
        let identity = Arc::new(IdentityVerifier::new(Arc::new(|_| None)).unwrap());
        let clock = Arc::new(std::sync::atomic::AtomicU64::new(now_ms()));
        let sharing_clock = clock.clone();
        let sharing = Arc::new(Sharing::new(friends, shares, identity).with_clock(Arc::new(
            move || sharing_clock.load(std::sync::atomic::Ordering::SeqCst),
        )));
        let owner =
            crate::owner::OwnerStore::for_test(&h.data_dir, &crate::gateway::tests::owner());
        let gateway = Arc::new(
            Gateway::new(
                crate::config::PublicUrls {
                    scheme: "http".into(),
                    base_domain: CORE.into(),
                    port: h.addr.port(),
                },
                owner,
                TOKEN.into(),
                h.members.clone(),
                h.manager.clone(),
                None,
                h.http.clone(),
                h.tunnel.clone(),
            )
            .with_sharing(Some(sharing.clone())),
        );
        let peer = SharePeer {
            user_id: friend.user_id,
            generation: friend.access_generation,
        };
        Self {
            h,
            gateway,
            sharing,
            peer,
            clock,
        }
    }
    fn grant(&self, expires_at_ms: Option<u64>) -> ShareGrant {
        self.sharing
            .shares
            .create(
                &ShareGrantUpdate {
                    plugin_id: "app.whoami".into(),
                    recipient: ShareRecipient::Friend {
                        user_id: self.peer.user_id.clone(),
                    },
                    expires_at_ms,
                    acknowledge_risk: true,
                },
                &self
                    .h
                    .manager
                    .share_offer(&PluginId::parse("app.whoami").unwrap()),
            )
            .unwrap()
    }
    async fn get(&self, host: &str, path: &str) -> Reply {
        self.get_with(host, path, &[]).await
    }
    async fn get_with(
        &self,
        host: &str,
        path: &str,
        extra: &[(&'static str, &'static str)],
    ) -> Reply {
        let mut req = Request::builder()
            .uri(path)
            .header("host", host)
            .header("authorization", format!("Bearer {TOKEN}"))
            .header("cookie", "wonton_core=forged; wonton_app=forged")
            .body(Body::empty())
            .unwrap();
        for (name, value) in extra {
            req.headers_mut().append(*name, value.parse().unwrap());
        }
        req.extensions_mut().insert(self.peer.clone());
        Reply::of(router(self.gateway.clone()).oneshot(req).await.unwrap()).await
    }
}

#[tokio::test]
async fn share_identity_never_opens_core_management_matrix_files_or_mcp() {
    let f = Fixture::new().await;
    f.grant(None);
    for (host, path) in [
        (CORE, "/_wonton/api/plugins"),
        (CORE, "/_wonton/api/friends"),
        (CORE, "/_wonton/files/file/content"),
        (CORE, "/_wonton/login"),
        ("matrix.wonton.localhost", "/_matrix/client/v3/sync"),
        ("mcp.wonton.internal", "/mcp"),
        ("auth.wonton.localhost", "/admin/clients"),
        ("auth.wonton.localhost", "/oauth2/token"),
    ] {
        let reply = f.get(host, path).await;
        assert_eq!(reply.status, StatusCode::FORBIDDEN);
        assert_eq!(reply.error_kind(), Some("share_required"));
    }
    assert_eq!(f.get(PLUGIN, "/").await.status, StatusCode::OK);
}

#[tokio::test]
async fn headers_cannot_manufacture_share_identity_and_each_request_checks_the_grant() {
    let f = Fixture::new().await;
    let req = Request::builder()
        .uri("/")
        .header("host", PLUGIN)
        .header("x-wonton-share-user", &f.peer.user_id)
        .header("remote-user", "owner")
        .body(Body::empty())
        .unwrap();
    let reply = Reply::of(router(f.gateway.clone()).oneshot(req).await.unwrap()).await;
    assert_eq!(reply.status, StatusCode::UNAUTHORIZED);
    assert_eq!(
        f.get(PLUGIN, "/").await.error_kind(),
        Some("share_required")
    );
    let grant = f.grant(None);
    assert_eq!(f.get(PLUGIN, "/").await.status, StatusCode::OK);
    assert!(
        f.sharing
            .friends
            .get(&f.peer.user_id)
            .unwrap()
            .last_opened_share_at_ms
            .is_some()
    );
    f.sharing.shares.revoke(&grant.id).unwrap();
    assert_eq!(f.get(PLUGIN, "/").await.error_kind(), Some("share_revoked"));
    let grant = f.grant(Some(now_ms() + 60_000));
    assert_eq!(f.get(PLUGIN, "/").await.status, StatusCode::OK);
    f.clock.store(
        grant.expires_at_ms.unwrap(),
        std::sync::atomic::Ordering::SeqCst,
    );
    assert_eq!(f.get(PLUGIN, "/").await.error_kind(), Some("share_expired"));
    f.sharing.friends.remove(&f.peer.user_id).unwrap();
    f.sharing
        .friends
        .accept(&f.peer.user_id, Some("Alice"), now_ms(), |_| false)
        .unwrap();
    f.grant(None);
    assert_eq!(f.get(PLUGIN, "/").await.error_kind(), Some("share_revoked"));
}

async fn upgrade(f: &Fixture) -> tokio::io::DuplexStream {
    let (client, server) = tokio::io::duplex(8192);
    tokio::spawn(super::handler::serve_io(
        f.gateway.clone(),
        f.peer.clone(),
        server,
    ));
    let mut client = client;
    client
        .write_all(
            format!(
                "GET /ws HTTP/1.1\r\nHost: {PLUGIN}\r\nConnection: upgrade\r\nUpgrade: echo\r\n\r\n"
            )
            .as_bytes(),
        )
        .await
        .unwrap();
    let mut response = Vec::new();
    while !response.ends_with(b"\r\n\r\n") {
        response.push(client.read_u8().await.unwrap());
    }
    assert!(response.starts_with(b"HTTP/1.1 101 "));
    client
}
async fn closed(mut client: tokio::io::DuplexStream) {
    let mut byte = [0];
    let result = tokio::time::timeout(Duration::from_secs(2), client.read(&mut byte))
        .await
        .unwrap();
    assert!(matches!(result, Ok(0) | Err(_)));
}

#[tokio::test]
async fn revocation_closes_all_upgrades_even_after_http_service_returns() {
    let f = Fixture::new().await;
    let grant = f.grant(None);
    let a = upgrade(&f).await;
    let b = upgrade(&f).await;
    f.sharing.shares.revoke(&grant.id).unwrap();
    tokio::join!(closed(a), closed(b));
}

#[tokio::test]
async fn a_new_grant_cannot_keep_an_old_websocket_alive_after_coalesced_revocation() {
    let f = Fixture::new().await;
    let grant = f.grant(None);
    let a = upgrade(&f).await;
    f.sharing.shares.revoke(&grant.id).unwrap();
    f.grant(None);
    closed(a).await;
    assert_eq!(f.get(PLUGIN, "/").await.status, StatusCode::OK);
}

#[tokio::test]
async fn overlapping_original_grants_keep_access_until_the_last_one_is_revoked() {
    let f = Fixture::new().await;
    let first = f.grant(None);
    let second = f.grant(None);
    let mut req = Request::builder()
        .method(Method::GET)
        .uri("/")
        .body(Body::empty())
        .unwrap();
    req.extensions_mut().insert(f.peer.clone());
    super::guard_request(&f.gateway, &mut req, "app.whoami").unwrap();
    let guard = req
        .extensions()
        .get::<super::AccessGuard>()
        .unwrap()
        .clone();
    f.sharing.shares.revoke(&first.id).unwrap();
    assert!(
        f.sharing
            .authorize(&f.peer, "app.whoami", &f.h.manager)
            .is_ok()
    );
    // The original second permit still authorizes; then its revocation must cancel.
    f.sharing.shares.revoke(&second.id).unwrap();
    tokio::time::timeout(Duration::from_secs(2), guard.cancelled())
        .await
        .unwrap();
}

#[tokio::test]
async fn expiry_closes_an_existing_upgrade_and_new_requests_refuse_at_the_exact_deadline() {
    let f = Fixture::new().await;
    let grant = f.grant(Some(now_ms() + 60_000));
    let client = upgrade(&f).await;
    f.clock.store(
        grant.expires_at_ms.unwrap(),
        std::sync::atomic::Ordering::SeqCst,
    );
    // A registry publication wakes the guard after advancing the injected wall clock.
    f.sharing
        .friends
        .update_display_name(&f.peer.user_id, Some("Alice updated"))
        .unwrap();
    closed(client).await;
    assert_eq!(f.get(PLUGIN, "/").await.error_kind(), Some("share_expired"));
}

async fn streaming_http(f: &Fixture) -> (tokio::io::DuplexStream, Arc<tokio::sync::Notify>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let hold = Arc::new(tokio::sync::Notify::new());
    let upstream_hold = hold.clone();
    tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut request = Vec::new();
        while !request.ends_with(b"\r\n\r\n") {
            request.push(stream.read_u8().await.unwrap());
        }
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 100000\r\n\r\nx")
            .await
            .unwrap();
        upstream_hold.notified().await;
    });
    let manifest = crate::plugin::PluginPackage::manifest_of(
        crate::gateway::tests::WHOAMI,
        &crate::plugin::languages::Languages::default(),
    )
    .unwrap();
    f.h.manager.register_for_test(
        manifest,
        crate::plugin::PluginStatus::Running,
        Some(addr.to_string()),
    );
    let (mut client, server) = tokio::io::duplex(8192);
    tokio::spawn(super::handler::serve_io(
        f.gateway.clone(),
        f.peer.clone(),
        server,
    ));
    client
        .write_all(format!("GET / HTTP/1.1\r\nHost: {PLUGIN}\r\n\r\n").as_bytes())
        .await
        .unwrap();
    let mut headers = Vec::new();
    while !headers.ends_with(b"\r\n\r\n") {
        headers.push(client.read_u8().await.unwrap());
    }
    assert!(headers.starts_with(b"HTTP/1.1 200 "));
    assert_eq!(client.read_u8().await.unwrap(), b'x');
    (client, hold)
}

#[tokio::test]
async fn revocation_resets_an_http_response_body_after_its_headers_were_forwarded() {
    let f = Fixture::new().await;
    let grant = f.grant(None);
    let (client, hold) = streaming_http(&f).await;
    f.sharing.shares.revoke(&grant.id).unwrap();
    closed(client).await;
    hold.notify_one();
}
