//! OIDC 登录：假扮一台只有 OAuth 2.0 API（MSC2965）、密码登录关着的 homeserver，连同它的授权服务器
//! （动态注册、令牌端点）。测的是核心生成授权地址、核对回调、换到令牌后存成槽位；浏览器那一段由测试拼回调代替。

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post, put},
};
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;
use url::Url;
use wonton_client::{
    Action, ClientCore, CoreError, MatrixLoginFailure, MatrixLoginMethod, MatrixLoginState,
    MatrixOidcCallback, MatrixOidcLogin, SecretString,
};
use wonton_protocol::ServerKind;

use super::{open, settled, slot_dir_gone, start};

/// 和核心发布的 `oidc_redirect_uri` 必须一致：matrix.org 只收 `client_uri` 域名倒写的 scheme。
const REDIRECT: &str = "io.github.wontonapp:/matrix-login";
const CLIENT_URI: &str = "https://wontonapp.github.io/";
const USER: &str = "@alice:oidc.example";

struct FakeHomeserver {
    url: String,
    /// 最近一次动态注册收到的客户端元数据。
    registered: Arc<Mutex<Option<Value>>>,
    task: JoinHandle<()>,
}

#[derive(Clone)]
struct Fake {
    base: Arc<String>,
    registered: Arc<Mutex<Option<Value>>>,
}

impl Drop for FakeHomeserver {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl FakeHomeserver {
    async fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let registered = Arc::new(Mutex::new(None));
        let fake = Fake {
            base: Arc::new(url.clone()),
            registered: registered.clone(),
        };
        let app = Router::new()
            .route("/_matrix/client/versions", get(versions))
            .route("/_matrix/client/v3/login", get(login_types))
            .route("/_matrix/client/v1/auth_metadata", get(metadata))
            .route("/oauth2/register", post(register))
            .route("/oauth2/token", post(token))
            .route("/_matrix/client/v3/account/whoami", get(whoami))
            .route("/_matrix/client/v3/devices/{device}", put(rename))
            .fallback(unrecognized)
            .with_state(fake);
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        Self {
            url,
            registered,
            task,
        }
    }
}

async fn versions() -> Json<Value> {
    Json(json!({
        "versions": ["v1.11", "v1.15"],
        "unstable_features": { "org.matrix.simplified_msc3575": true }
    }))
}

/// 只有旧式 SSO：不能算成 OIDC，也没有密码。
async fn login_types() -> Json<Value> {
    Json(json!({ "flows": [{ "type": "m.login.sso" }] }))
}

async fn metadata(State(Fake { base, .. }): State<Fake>) -> Json<Value> {
    Json(json!({
        "issuer": format!("{base}/"),
        "authorization_endpoint": format!("{base}/oauth2/authorize"),
        "token_endpoint": format!("{base}/oauth2/token"),
        "registration_endpoint": format!("{base}/oauth2/register"),
        "revocation_endpoint": format!("{base}/oauth2/revoke"),
        "response_types_supported": ["code"],
        "response_modes_supported": ["query", "fragment"],
        "grant_types_supported": ["authorization_code", "refresh_token"],
        "code_challenge_methods_supported": ["S256"],
    }))
}

async fn register(State(fake): State<Fake>, Json(body): Json<Value>) -> impl IntoResponse {
    *fake.registered.lock().unwrap() = Some(body);
    (
        StatusCode::CREATED,
        Json(json!({ "client_id": "fake-client" })),
    )
}

async fn token() -> Json<Value> {
    Json(json!({
        "access_token": "fake-access",
        "token_type": "Bearer",
        "refresh_token": "fake-refresh",
        "expires_in": 3600,
    }))
}

async fn whoami() -> Json<Value> {
    Json(json!({ "user_id": USER }))
}

async fn rename() -> Json<Value> {
    Json(json!({}))
}

async fn unrecognized() -> impl IntoResponse {
    (
        StatusCode::NOT_FOUND,
        Json(json!({ "errcode": "M_UNRECOGNIZED", "error": "not in the fake" })),
    )
}

fn oidc(login_id: &str, redirect: &str) -> Action {
    Action::StartMatrixOidc {
        login: MatrixOidcLogin {
            login_id: login_id.to_owned(),
            redirect_uri: redirect.to_owned(),
            device_name: "测试手机".to_owned(),
        },
    }
}

fn callback(login_id: &str, url: String) -> Action {
    Action::CompleteMatrixOidc {
        callback: MatrixOidcCallback {
            login_id: login_id.to_owned(),
            callback_url: SecretString::new(url),
        },
    }
}

/// 发现、开始 OIDC，返回流程 ID 和授权地址里的参数。
async fn awaiting(core: &ClientCore, server: &FakeHomeserver) -> (String, HashMap<String, String>) {
    start(core, &server.url);
    let ready = settled(core).await;
    assert_eq!(ready.state, MatrixLoginState::Ready, "{ready:?}");
    assert_eq!(
        ready.methods,
        [MatrixLoginMethod::Oidc],
        "旧式 SSO 不算 OIDC"
    );
    assert_eq!(ready.oidc_redirect_uri, REDIRECT);
    core.dispatch(oidc(&ready.login_id, &ready.oidc_redirect_uri))
        .unwrap();
    let login = settled(core).await;
    let MatrixLoginState::AwaitingOidc { authorization_url } = login.state else {
        panic!("没拿到授权地址：{login:?}");
    };
    let url = Url::parse(authorization_url.expose()).unwrap();
    assert!(
        url.as_str()
            .starts_with(&format!("{}/oauth2/authorize?", server.url)),
        "{url}"
    );
    let query: HashMap<String, String> = url.query_pairs().into_owned().collect();
    assert_eq!(query["client_id"], "fake-client");
    assert_eq!(query["redirect_uri"], REDIRECT);
    let registered = server
        .registered
        .lock()
        .unwrap()
        .clone()
        .expect("动态注册过");
    assert_eq!(registered["client_uri"], CLIENT_URI, "{registered}");
    assert_eq!(
        registered["redirect_uris"],
        json!([REDIRECT]),
        "{registered}"
    );
    assert_eq!(registered["application_type"], "native", "{registered}");
    assert_eq!(query["code_challenge_method"], "S256");
    assert!(!query["state"].is_empty());
    (ready.login_id, query)
}

/// 授权地址带着动态注册的客户端、PKCE 和 state；回调回来换到令牌，账号存成槽位并换过去。
#[tokio::test]
async fn an_oidc_login_builds_the_authorization_url_and_finishes_into_a_slot() {
    let server = FakeHomeserver::start().await;
    let dir = tempfile::tempdir().unwrap();
    let core = open(dir.path());
    let (login_id, query) = awaiting(&core, &server).await;

    // iOS 从 App 外面交回来的写法是 `scheme:///path`，和注册的 `scheme:/path` 是同一个地址。
    let state = &query["state"];
    core.dispatch(callback(
        &login_id,
        format!("io.github.wontonapp:///matrix-login?code=fake-code&state={state}"),
    ))
    .unwrap();
    let done = settled(&core).await;
    let MatrixLoginState::Succeeded { slot_id, account } = done.state else {
        panic!("登录没成功：{done:?}");
    };
    assert_eq!(account.user_id, USER);
    assert!(account.homeserver.starts_with(&server.url), "{account:?}");
    let row = core
        .servers()
        .servers
        .into_iter()
        .find(|row| row.id == slot_id)
        .expect("新槽位在列表里");
    assert!(row.current);
    assert_eq!(row.kind, ServerKind::Matrix { account });
}

/// 只接受平台注册的回调地址。
#[tokio::test]
async fn an_unregistered_redirect_is_refused() {
    let server = FakeHomeserver::start().await;
    let dir = tempfile::tempdir().unwrap();
    let core = open(dir.path());
    start(&core, &server.url);
    let ready = settled(&core).await;
    assert!(matches!(
        core.dispatch(oidc(&ready.login_id, "https://evil.example/callback")),
        Err(CoreError::InvalidInput { .. })
    ));
    assert_eq!(
        core.servers().matrix_login.unwrap().state,
        MatrixLoginState::Ready
    );
}

/// 旧流程的回调被拒，不动新流程；state 对不上的回调让这次登录失败；被取消的流程收不了迟到的回调。
#[tokio::test]
async fn mismatched_callbacks_are_rejected() {
    let server = FakeHomeserver::start().await;
    let dir = tempfile::tempdir().unwrap();
    let core = open(dir.path());
    let before = core.servers().servers;
    let (old_id, old_query) = awaiting(&core, &server).await;
    let (new_id, _) = awaiting(&core, &server).await;

    let stale = callback(
        &old_id,
        format!("{REDIRECT}?code=c&state={}", old_query["state"]),
    );
    assert!(matches!(
        core.dispatch(stale),
        Err(CoreError::InvalidInput { .. })
    ));
    let current = core.servers().matrix_login.unwrap();
    assert_eq!(current.login_id, new_id);
    assert!(matches!(
        current.state,
        MatrixLoginState::AwaitingOidc { .. }
    ));

    core.dispatch(callback(
        &new_id,
        format!("{REDIRECT}?code=c&state={}", old_query["state"]),
    ))
    .unwrap();
    assert_eq!(
        settled(&core).await.state,
        MatrixLoginState::Failed {
            reason: MatrixLoginFailure::InvalidCallback
        }
    );
    assert_eq!(core.servers().servers, before);

    let (cancelled, query) = awaiting(&core, &server).await;
    core.dispatch(Action::CancelMatrixLogin {
        login_id: cancelled.clone(),
    })
    .unwrap();
    let late = callback(
        &cancelled,
        format!("{REDIRECT}?code=c&state={}", query["state"]),
    );
    assert!(matches!(
        core.dispatch(late),
        Err(CoreError::InvalidInput { .. })
    ));
    assert_eq!(
        core.servers().matrix_login.unwrap().state,
        MatrixLoginState::Cancelled
    );
    assert_eq!(core.servers().servers, before);
    for slot in ["2", "3", "4"] {
        slot_dir_gone(dir.path(), slot).await;
    }
}

/// 发往别的地址的回调、授权服务器拒绝（用户在浏览器里点了拒绝）都如实失败。
#[tokio::test]
async fn foreign_and_denied_callbacks_fail_with_their_category() {
    let server = FakeHomeserver::start().await;
    let dir = tempfile::tempdir().unwrap();
    let core = open(dir.path());

    let (login_id, query) = awaiting(&core, &server).await;
    core.dispatch(callback(
        &login_id,
        format!("https://evil.example/cb?code=c&state={}", query["state"]),
    ))
    .unwrap();
    assert_eq!(
        settled(&core).await.state,
        MatrixLoginState::Failed {
            reason: MatrixLoginFailure::InvalidCallback
        }
    );

    let (login_id, query) = awaiting(&core, &server).await;
    core.dispatch(callback(
        &login_id,
        format!("{REDIRECT}?error=access_denied&state={}", query["state"]),
    ))
    .unwrap();
    assert_eq!(
        settled(&core).await.state,
        MatrixLoginState::Failed {
            reason: MatrixLoginFailure::OidcRejected
        }
    );
}

/// 壳在自己的主线程上取消、交回对不上的回调，那里没有 tokio 运行时：登录用的客户端在那时被丢掉，不能让核心崩溃。
#[tokio::test]
async fn cancelling_or_failing_from_a_thread_without_a_runtime_does_not_crash() {
    let server = FakeHomeserver::start().await;
    let dir = tempfile::tempdir().unwrap();
    let core = open(dir.path());

    let (login_id, _) = awaiting(&core, &server).await;
    on_plain_thread(&core, Action::CancelMatrixLogin { login_id });
    assert_eq!(
        core.servers().matrix_login.unwrap().state,
        MatrixLoginState::Cancelled
    );

    let (login_id, _) = awaiting(&core, &server).await;
    on_plain_thread(
        &core,
        callback(&login_id, format!("{REDIRECT}?code=x&state=y")),
    );
    assert_eq!(
        core.servers().matrix_login.unwrap().state,
        MatrixLoginState::Failed {
            reason: MatrixLoginFailure::InvalidCallback
        }
    );
}

fn on_plain_thread(core: &Arc<ClientCore>, action: Action) {
    let core = core.clone();
    std::thread::spawn(move || core.dispatch(action))
        .join()
        .expect("核心在没有运行时的线程上崩溃")
        .unwrap();
}
