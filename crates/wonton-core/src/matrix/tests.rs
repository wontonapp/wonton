//! 核心准备 Tuwunel 管理账号的行为：在一个假的 Tuwunel 上验证注册、复用、失效后重新登录和管理员权限。

use std::collections::BTreeSet;
use std::os::unix::fs::PermissionsExt;
use std::sync::{Arc, Mutex};

use aws_lc_rs::encoding::AsDer;
use aws_lc_rs::hmac;
use aws_lc_rs::signature::{Ed25519KeyPair, KeyPair};
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::routing::{get, post};
use axum::{Json, Router};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use jsonwebtoken::DecodingKey;
use serde_json::{Value, json};
use tokio::net::TcpListener;

use super::Matrix;
use super::issuer::tests::verify;
use crate::config::MatrixConfig;
use crate::http::HttpClient;

const SECRET: &str = "shared-secret";

/// 假 Tuwunel 记下的状态：管理账号在不在、是不是服务器管理员，哪些令牌有效。
#[derive(Default)]
pub(crate) struct Tuwunel {
    registered: bool,
    admin: bool,
    registrations: usize,
    tokens: BTreeSet<String>,
    next: usize,
}

/// 假的 Tuwunel：只实现核心用到的几个接口；`/_matrix/client/versions` 也给网关的转发测试用。
#[derive(Clone)]
pub(crate) struct Fake {
    state: Arc<Mutex<Tuwunel>>,
    /// 验签核心签的登录令牌用的公钥。
    pub(crate) key: Arc<DecodingKey>,
}

impl Fake {
    fn issue(&self) -> String {
        let mut state = self.state.lock().unwrap();
        state.next += 1;
        let token = format!("token-{}", state.next);
        state.tokens.insert(token.clone());
        token
    }
}

fn matrix_error(status: StatusCode, errcode: &str) -> (StatusCode, Json<Value>) {
    (status, Json(json!({"errcode": errcode, "error": errcode})))
}

async fn register(State(fake): State<Fake>, Json(body): Json<Value>) -> (StatusCode, Json<Value>) {
    let key = hmac::Key::new(hmac::HMAC_SHA1_FOR_LEGACY_USE_ONLY, SECRET.as_bytes());
    let field = |name: &str| body[name].as_str().unwrap_or_default().to_owned();
    let message = [
        field("nonce"),
        field("username"),
        field("password"),
        "admin".into(),
    ]
    .join("\0");
    let mac: String = hmac::sign(&key, message.as_bytes())
        .as_ref()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    if field("mac") != mac || body["admin"] != true {
        return matrix_error(StatusCode::FORBIDDEN, "M_FORBIDDEN");
    }
    {
        let mut state = fake.state.lock().unwrap();
        if state.registered {
            return matrix_error(StatusCode::BAD_REQUEST, "M_USER_IN_USE");
        }
        state.registered = true;
        state.admin = true;
        state.registrations += 1;
    }
    let token = fake.issue();
    (
        StatusCode::OK,
        Json(json!({"access_token": token, "user_id": "@wonton-core:test"})),
    )
}

async fn login(State(fake): State<Fake>, Json(body): Json<Value>) -> (StatusCode, Json<Value>) {
    let claims = verify(body["token"].as_str().unwrap_or_default(), &fake.key);
    let user = format!("@{}:test", claims["sub"].as_str().unwrap());
    let token = fake.issue();
    let reply = json!({"user_id": user, "device_id": "DEVICE", "access_token": token});
    (StatusCode::OK, Json(reply))
}

fn bearer(headers: &HeaderMap) -> &str {
    headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .unwrap_or_default()
}

async fn whoami(State(fake): State<Fake>, headers: HeaderMap) -> (StatusCode, Json<Value>) {
    if fake.state.lock().unwrap().tokens.contains(bearer(&headers)) {
        (
            StatusCode::OK,
            Json(json!({"user_id": "@wonton-core:test"})),
        )
    } else {
        matrix_error(StatusCode::UNAUTHORIZED, "M_UNKNOWN_TOKEN")
    }
}

/// 管理接口：令牌有效、账号又是服务器管理员才行，和 Tuwunel 一样对普通账号回 403。
async fn devices(State(fake): State<Fake>, headers: HeaderMap) -> (StatusCode, Json<Value>) {
    let state = fake.state.lock().unwrap();
    if !state.tokens.contains(bearer(&headers)) {
        matrix_error(StatusCode::UNAUTHORIZED, "M_UNKNOWN_TOKEN")
    } else if !state.admin {
        matrix_error(StatusCode::FORBIDDEN, "M_FORBIDDEN")
    } else {
        (StatusCode::OK, Json(json!({"devices": [], "total": 0})))
    }
}

/// 起一个假 Tuwunel，返回它和一份指向它的配置（签发私钥是新生成的）。
pub(crate) async fn start() -> (Fake, MatrixConfig) {
    let pair = Ed25519KeyPair::generate().unwrap();
    let der = pair.to_pkcs8().unwrap();
    let public = pair.public_key().as_der().unwrap();
    let fake = Fake {
        state: Arc::default(),
        key: Arc::new(DecodingKey::from_ed_der(public.as_ref())),
    };
    let app = Router::new()
        .route(
            "/_matrix/client/versions",
            get(|| async { Json(json!({"versions": ["v1.11"]})) }),
        )
        .route(
            "/_synapse/admin/v1/register",
            get(|| async { Json(json!({"nonce": "n1"})) }).post(register),
        )
        .route("/_matrix/client/v3/login", post(login))
        .route("/_matrix/client/v3/account/whoami", get(whoami))
        .route("/_synapse/admin/v2/users/{user}/devices", get(devices))
        // 其余路径：回 404，带上收到的原始路径，测试据此确认网关原样转发。
        .fallback(|uri: axum::http::Uri| async move {
            let body = json!({"errcode": "M_UNRECOGNIZED", "path": uri.path()});
            (StatusCode::NOT_FOUND, Json(body))
        })
        .with_state(fake.clone());
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let body = STANDARD.encode(der.as_ref());
    let config = MatrixConfig {
        url: format!("http://{addr}"),
        server_name: "test".into(),
        jwt_key: format!("-----BEGIN PRIVATE KEY-----\n{body}\n-----END PRIVATE KEY-----\n"),
        registration_secret: SECRET.into(),
    };
    (fake, config)
}

#[tokio::test]
async fn the_admin_account_is_registered_once_and_its_token_kept_private() {
    let (fake, config) = start().await;
    let dir = tempfile::tempdir().unwrap();
    let first = Matrix::connect(&config, dir.path(), HttpClient::new())
        .await
        .unwrap();
    let saved = dir.path().join("matrix-admin-token");
    let mode = std::fs::metadata(&saved).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600);
    assert_eq!(first.admin_user_id(), "@wonton-core:test");

    // 核心重启：令牌还有效就沿用，不再注册。
    let again = Matrix::connect(&config, dir.path(), HttpClient::new())
        .await
        .unwrap();
    assert_eq!(again.admin_token, first.admin_token);
    assert_eq!(fake.state.lock().unwrap().registrations, 1);

    // 令牌失效（比如 Tuwunel 吊销了它）：账号已经在了，用核心签的 JWT 登录拿新令牌。
    fake.state.lock().unwrap().tokens.clear();
    let relogged = Matrix::connect(&config, dir.path(), HttpClient::new())
        .await
        .unwrap();
    assert_ne!(relogged.admin_token, first.admin_token);
    assert_eq!(fake.state.lock().unwrap().registrations, 1);
    let kept = std::fs::read_to_string(&saved).unwrap();
    assert_eq!(kept, relogged.admin_token);
}

#[tokio::test]
async fn a_wrong_shared_secret_is_refused_by_tuwunel() {
    let (_fake, mut config) = start().await;
    config.registration_secret = "wrong".into();
    let dir = tempfile::tempdir().unwrap();
    let Err(error) = Matrix::connect(&config, dir.path(), HttpClient::new()).await else {
        panic!("共享密钥不对时不能拿到管理令牌");
    };
    assert_eq!(error.errcode(), "M_FORBIDDEN", "{error}");
    assert!(!dir.path().join("matrix-admin-token").exists());
}

#[tokio::test]
async fn an_account_that_is_not_a_server_admin_stops_the_core() {
    // 管理账号的用户名先被 JWT 登录建成了普通账号：注册报 M_USER_IN_USE，登录能拿到令牌，但调不了管理接口。
    let (fake, config) = start().await;
    fake.state.lock().unwrap().registered = true;
    let dir = tempfile::tempdir().unwrap();
    let Err(error) = Matrix::connect(&config, dir.path(), HttpClient::new()).await else {
        panic!("管理账号不是服务器管理员时核心不能启动");
    };
    assert_eq!(error.errcode(), "M_FORBIDDEN", "{error}");
    assert!(error.to_string().contains("服务器管理员"), "{error}");
}
