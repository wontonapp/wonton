use std::{
    net::Ipv4Addr,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};

use iroh::{
    Endpoint,
    endpoint::{Connection, RecvStream, SendStream, presets},
    protocol::Router,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    sync::{Notify, mpsc},
    time::timeout,
};
use wonton_protocol::{
    GatewayRefusal, MatrixOpenIdToken, SecretString, ShareIdentityFailure, ShareReply,
    ShareRequest, SharedPlugins,
};

use super::*;

const WAIT: Duration = Duration::from_secs(5);
const USER: &str = "@verified:identity.test";

type Forwarded = (String, Connection, ShareStream);

struct Handler {
    authenticated: AtomicUsize,
    forwarded: mpsc::UnboundedSender<Forwarded>,
    seen: mpsc::UnboundedSender<()>,
    gate: Option<Arc<Notify>>,
    wrong_list_identity: AtomicBool,
    generation: AtomicUsize,
}

impl ShareHandler for Handler {
    fn authenticate(
        &self,
        token: MatrixOpenIdToken,
    ) -> BoxFuture<'_, Result<AuthenticatedShares, ShareRefusal>> {
        Box::pin(async move {
            self.authenticated.fetch_add(1, Ordering::SeqCst);
            self.seen.send(()).unwrap();
            if let Some(gate) = &self.gate {
                gate.notified().await;
            }
            if token.access_token.expose() != "valid-credential" {
                return Err(ShareRefusal::identity(ShareIdentityFailure::InvalidToken));
            }
            // Test verifier deliberately returns an identity not inferable from input.
            Ok(AuthenticatedShares {
                shares: shares(USER),
                generation: self.generation.load(Ordering::SeqCst) as u64,
            })
        })
    }
    fn list<'a>(
        &'a self,
        session: &'a ShareSession,
    ) -> BoxFuture<'a, Result<SharedPlugins, ShareRefusal>> {
        Box::pin(async move {
            let user = session.user_id();
            if session.generation != self.generation.load(Ordering::SeqCst) as u64 {
                return Err(ShareRefusal {
                    refusal: GatewayRefusal::ShareRevoked,
                    identity: None,
                });
            }
            assert_eq!(user, USER);
            Ok(shares(if self.wrong_list_identity.load(Ordering::SeqCst) {
                "@different:identity.test"
            } else {
                user
            }))
        })
    }
    fn forward<'a>(
        &'a self,
        session: &'a ShareSession,
        conn: Connection,
        stream: ShareStream,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            if session.generation == self.generation.load(Ordering::SeqCst) as u64 {
                self.forwarded
                    .send((session.user_id().to_owned(), conn, stream))
                    .unwrap();
            }
        })
    }
}

fn shares(user: &str) -> SharedPlugins {
    SharedPlugins {
        user_id: user.into(),
        plugins: vec![],
    }
}
fn token(value: &str) -> MatrixOpenIdToken {
    MatrixOpenIdToken {
        access_token: SecretString::new(value),
        matrix_server_name: "input.test".into(),
        expires_in: 3600,
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

struct Setup {
    router: Router,
    phone: Endpoint,
    handler: Arc<Handler>,
    forwards: mpsc::UnboundedReceiver<Forwarded>,
    seen: mpsc::UnboundedReceiver<()>,
}

impl Setup {
    async fn new(gate: Option<Arc<Notify>>) -> Self {
        Self::with_limits(gate, ShareLimits::default()).await
    }

    async fn with_limits(gate: Option<Arc<Notify>>, limits: ShareLimits) -> Self {
        let (forwarded, forwards) = mpsc::unbounded_channel();
        let (seen_tx, seen) = mpsc::unbounded_channel();
        let handler = Arc::new(Handler {
            authenticated: AtomicUsize::new(0),
            forwarded,
            seen: seen_tx,
            gate,
            wrong_list_identity: AtomicBool::new(false),
            generation: AtomicUsize::new(7),
        });
        let protocol = ShareProtocol::new(handler.clone(), limits);
        let router = Router::builder(endpoint().await)
            .accept(SHARE_ALPN, protocol)
            .spawn();
        Self {
            router,
            phone: endpoint().await,
            handler,
            forwards,
            seen,
        }
    }
    async fn raw(&self) -> Connection {
        self.phone
            .connect(self.router.endpoint().addr(), SHARE_ALPN)
            .await
            .unwrap()
    }
    async fn client(&self) -> ShareClient {
        ShareClient::connect(
            self.phone.clone(),
            self.router.endpoint().addr(),
            token("valid-credential"),
        )
        .await
        .unwrap()
    }
    async fn stop(self) {
        self.phone.close().await;
        self.router.shutdown().await.unwrap();
    }
}

async fn request(conn: &Connection, request: &ShareRequest) -> ShareReply {
    let (mut send, mut recv) = start_request(conn, request).await;
    let reply = timeout(WAIT, frame::read(&mut recv, frame::MAX_REPLY))
        .await
        .unwrap()
        .unwrap();
    send.finish().ok();
    reply
}

async fn start_request(conn: &Connection, request: &ShareRequest) -> (SendStream, RecvStream) {
    let (mut send, recv) = conn.open_bi().await.unwrap();
    send.write_all(&[frame::CONTROL]).await.unwrap();
    frame::write(&mut send, request, frame::MAX_REQUEST)
        .await
        .unwrap();
    (send, recv)
}

#[tokio::test]
async fn invalid_credentials_are_categorized_and_never_forwarded() {
    let mut setup = Setup::new(None).await;
    let result = ShareClient::connect(
        setup.phone.clone(),
        setup.router.endpoint().addr(),
        token("invalid-secret-that-must-not-escape"),
    )
    .await
    .unwrap_err();
    assert_eq!(
        result,
        ShareError::Refused(ShareRefusal::identity(ShareIdentityFailure::InvalidToken))
    );
    assert!(setup.forwards.try_recv().is_err());
    assert_eq!(setup.handler.authenticated.load(Ordering::SeqCst), 1);
    setup.stop().await;
}

#[tokio::test]
async fn preauth_list_and_raw_http_never_gain_forwarding() {
    let mut setup = Setup::new(None).await;
    let conn = setup.raw().await;
    let reply = request(&conn, &ShareRequest::List).await;
    assert!(matches!(
        reply,
        ShareReply::Refused {
            refusal: GatewayRefusal::ShareIdentity,
            ..
        }
    ));
    conn.close(0u32.into(), b"done");
    let conn = setup.raw().await;
    let (mut send, _) = conn.open_bi().await.unwrap();
    send.write_all(b"\0GET / HTTP/1.1\r\nhost: app.test\r\n\r\n")
        .await
        .unwrap();
    timeout(WAIT, conn.closed()).await.unwrap();
    assert!(setup.forwards.try_recv().is_err());
    assert_eq!(setup.handler.authenticated.load(Ordering::SeqCst), 0);
    setup.stop().await;
}

#[tokio::test]
async fn racing_forward_during_userinfo_closes_connection_without_forwarding() {
    let gate = Arc::new(Notify::new());
    let mut setup = Setup::new(Some(gate.clone())).await;
    let conn = setup.raw().await;
    let _auth = start_request(
        &conn,
        &ShareRequest::Authenticate {
            openid: token("valid-credential"),
        },
    )
    .await;
    timeout(WAIT, setup.seen.recv()).await.unwrap().unwrap();
    let (mut send, _) = conn.open_bi().await.unwrap();
    send.write_all(b"\0GET / HTTP/1.1\r\n\r\n").await.unwrap();
    timeout(WAIT, conn.closed()).await.unwrap();
    gate.notify_one();
    assert!(setup.forwards.try_recv().is_err());
    setup.stop().await;
}

#[tokio::test]
async fn verified_identity_is_stable_for_list_and_forward_and_cannot_be_reauthenticated() {
    let mut setup = Setup::new(None).await;
    let conn = setup.raw().await;
    assert!(matches!(request(&conn, &ShareRequest::Authenticate {
        openid: token("valid-credential") }).await, ShareReply::Shares { shares: found } if found.user_id == USER));
    assert!(matches!(
        request(
            &conn,
            &ShareRequest::Authenticate {
                openid: token("another-valid-identity")
            }
        )
        .await,
        ShareReply::Refused { .. }
    ));
    assert!(matches!(request(&conn, &ShareRequest::List).await,
        ShareReply::Shares { shares: found } if found.user_id == USER));
    let (mut send, _recv) = conn.open_bi().await.unwrap();
    send.write_all(&[frame::FORWARD]).await.unwrap();
    let (user, server_conn, stream) = timeout(WAIT, setup.forwards.recv()).await.unwrap().unwrap();
    assert_eq!(user, USER);
    assert_eq!(stream.user_id(), USER);
    assert_eq!(stream.remote_id(), setup.phone.id());
    assert_eq!(stream.connection_id(), server_conn.stable_id());
    assert_eq!(setup.handler.authenticated.load(Ordering::SeqCst), 1);
    conn.close(0u32.into(), b"done");
    drop(stream);
    setup.stop().await;
}

#[tokio::test]
async fn client_list_reuses_connection_and_close_breaks_active_duplexes() {
    let mut setup = Setup::new(None).await;
    let client = setup.client().await;
    assert_eq!(client.list().await.unwrap().user_id, USER);
    let mut outgoing = client.open_forward().await.unwrap();
    let (_, conn, mut incoming) = timeout(WAIT, setup.forwards.recv()).await.unwrap().unwrap();
    outgoing.write_all(b"request").await.unwrap();
    let mut data = [0; 7];
    incoming.read_exact(&mut data).await.unwrap();
    assert_eq!(&data, b"request");
    client.close();
    timeout(WAIT, conn.closed()).await.unwrap();
    timeout(WAIT, client.closed()).await.unwrap();
    let mut byte = [0];
    assert!(
        timeout(WAIT, incoming.read(&mut byte))
            .await
            .unwrap()
            .is_err()
    );
    assert!(matches!(
        client.open_forward().await,
        Err(ShareError::Closed)
    ));
    assert_eq!(setup.handler.authenticated.load(Ordering::SeqCst), 1);
    setup.stop().await;
}

#[tokio::test]
async fn dropping_duplex_resets_peer_instead_of_clean_eof() {
    let mut setup = Setup::new(None).await;
    let client = setup.client().await;
    let outgoing = client.open_forward().await.unwrap();
    let (_, _, mut incoming) = timeout(WAIT, setup.forwards.recv()).await.unwrap().unwrap();
    drop(outgoing);
    let mut byte = [0];
    assert!(
        timeout(WAIT, incoming.read(&mut byte))
            .await
            .unwrap()
            .is_err()
    );
    client.close();
    setup.stop().await;
}

#[tokio::test]
async fn server_rejects_changed_list_identity_without_rebinding_session() {
    let setup = Setup::new(None).await;
    let client = setup.client().await;
    setup
        .handler
        .wrong_list_identity
        .store(true, Ordering::SeqCst);
    assert!(client.list().await.is_err());
    setup
        .handler
        .wrong_list_identity
        .store(false, Ordering::SeqCst);
    assert_eq!(client.list().await.unwrap().user_id, USER);
    assert_eq!(client.user_id(), USER);
    client.close();
    setup.stop().await;
}

mod framing;
mod hostile;
mod hub;
mod proxy;
