//! 恢复与设备列表刷新并行：旧检查的服务器应答不能盖掉已经导入的备份密钥。

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;

use axum::{
    Router,
    body::{Body, to_bytes},
    extract::{Request, State},
    response::Response,
};
use matrix_sdk::{
    Client,
    encryption::{EncryptionSettings, recovery::RecoveryState},
    reqwest,
};
use tokio::{net::TcpListener, sync::watch, task::JoinHandle};
use wonton_matrix::testkit::{TestServer, eventually, unique};
use wonton_matrix::{Recovery, Verification};

#[tokio::test]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn an_older_device_query_cannot_overwrite_completed_recovery() {
    let server = TestServer::start().await;
    let user = unique("owner");
    let first = server.login(&user, "first").await;
    first.start_sync().await.unwrap();
    eventually("first identity ready", first.encryption_states(), |state| {
        (state.verification == Verification::Verified).then_some(())
    })
    .await;
    let key = first.enable_recovery().await.unwrap();
    assert_eq!(first.encryption_state().recovery, Recovery::Enabled);

    let relay = RecoveryRelay::start(&server.url).await;
    let second = Client::builder()
        .homeserver_url(&relay.url)
        .with_encryption_settings(EncryptionSettings {
            auto_enable_cross_signing: true,
            ..Default::default()
        })
        .build()
        .await
        .unwrap();
    second
        .matrix_auth()
        .login_custom(
            "org.matrix.login.jwt",
            [("token".to_owned(), server.issuer.login_token(&user).into())]
                .into_iter()
                .collect(),
        )
        .unwrap()
        .await
        .unwrap();
    second
        .encryption()
        .wait_for_e2ee_initialization_tasks()
        .await;
    let encryption = second.encryption();
    let recovery = encryption.recovery();
    assert_eq!(recovery.state(), RecoveryState::Incomplete);

    relay.state.backup.arm();
    let recovering = recovery.recover(&key);
    tokio::pin!(recovering);
    tokio::select! {
        () = relay.state.backup.held() => {},
        result = &mut recovering => panic!("recovery must reach backup barrier: {result:?}"),
    }
    assert_eq!(recovery.state(), RecoveryState::Incomplete);
    relay.state.check.arm();
    let checking = encryption.request_user_identity(second.user_id().unwrap());
    tokio::pin!(checking);
    tokio::select! {
        () = relay.state.check.held() => {},
        result = &mut recovering => panic!("backup response is still held: {result:?}"),
        result = &mut checking => panic!("device query must reach state-check barrier: {result:?}"),
    }
    relay.state.backup.release.send_replace(true);
    tokio::select! {
        result = &mut recovering => result.unwrap(),
        result = &mut checking => panic!("old check must stay held: {result:?}"),
    }
    assert!(encryption.backups().are_enabled().await);
    assert!(
        encryption
            .cross_signing_status()
            .await
            .unwrap()
            .is_complete()
    );
    assert_eq!(recovery.state(), RecoveryState::Enabled);
    // 等的是同一个旧查询的完整返回；另发 keys/query 会重新计算状态，掩盖旧检查的覆盖。
    relay.state.check.release.send_replace(true);
    checking.await.unwrap();
    assert_eq!(recovery.state(), RecoveryState::Enabled);
}

struct RecoveryRelay {
    url: String,
    state: Arc<RelayState>,
    task: JoinHandle<()>,
}

struct RelayState {
    upstream: String,
    http: reqwest::Client,
    backup: Barrier,
    check: Barrier,
}

impl RecoveryRelay {
    async fn start(upstream: &str) -> Self {
        let state = Arc::new(RelayState {
            upstream: upstream.to_owned(),
            http: reqwest::Client::new(),
            backup: Barrier::default(),
            check: Barrier::default(),
        });
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let app = Router::new().fallback(forward).with_state(state.clone());
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        Self { url, state, task }
    }
}

impl Drop for RecoveryRelay {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn forward(State(state): State<Arc<RelayState>>, request: Request) -> Response {
    let (parts, body) = request.into_parts();
    let path = parts.uri.path();
    let gate = if parts.method == "GET" && path == "/_matrix/client/v3/room_keys/version" {
        Some(&state.backup)
    } else if parts.method == "GET" && path.ends_with("/account_data/m.key_backup") {
        Some(&state.check)
    } else {
        None
    };
    let body = to_bytes(body, usize::MAX).await.unwrap();
    let upstream = state
        .http
        .request(parts.method, format!("{}{}", state.upstream, parts.uri))
        .headers(parts.headers)
        .body(body)
        .send()
        .await
        .unwrap();
    let status = upstream.status();
    let headers = upstream.headers().clone();
    let body = upstream.bytes().await.unwrap();
    if let Some(gate) = gate.filter(|gate| gate.armed.swap(false, Ordering::SeqCst)) {
        gate.arrived.send_replace(true);
        let mut release = gate.release.subscribe();
        release.wait_for(|released| *released).await.unwrap();
    }
    let mut response = Response::new(Body::from(body));
    *response.status_mut() = status;
    *response.headers_mut() = headers;
    response.headers_mut().remove("transfer-encoding");
    response
}

struct Barrier {
    armed: AtomicBool,
    arrived: watch::Sender<bool>,
    release: watch::Sender<bool>,
}

impl Default for Barrier {
    fn default() -> Self {
        Self {
            armed: AtomicBool::new(false),
            arrived: watch::Sender::new(false),
            release: watch::Sender::new(false),
        }
    }
}

impl Barrier {
    fn arm(&self) {
        self.armed.store(true, Ordering::SeqCst);
    }

    async fn held(&self) {
        let mut arrived = self.arrived.subscribe();
        tokio::time::timeout(
            Duration::from_secs(60),
            arrived.wait_for(|arrived| *arrived),
        )
        .await
        .expect("request must reach recovery barrier")
        .unwrap();
    }
}
