use crate::{
    admin::{self, Command},
    auth::canonical,
    config::{Config, Relay},
    service::App,
    store::{Store, hex, now, random_token},
};
use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    routing::{get, post},
};
use iroh::SecretKey;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};
use tokio::sync::Mutex;

#[path = "tests_metrics.rs"]
mod metric_tests;

#[derive(Default)]
struct Upstream {
    next: AtomicUsize,
    patches: Mutex<Vec<Value>>,
    metrics: Mutex<String>,
}

async fn acme_register(State(state): State<Arc<Upstream>>) -> Json<Value> {
    let n = state.next.fetch_add(1, Ordering::SeqCst);
    Json(
        json!({"username":format!("user-{n}"),"password":format!("password-{n}"),"subdomain":format!("uuid-{n}"),"fulldomain":format!("uuid-{n}.auth.homes.test")}),
    )
}

async fn dns_patch(State(state): State<Arc<Upstream>>, Json(value): Json<Value>) -> StatusCode {
    state.patches.lock().await.push(value);
    StatusCode::NO_CONTENT
}

async fn serve(router: Router) -> (String, tokio::task::JoinHandle<()>) {
    let socket = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", socket.local_addr().unwrap());
    let task = tokio::spawn(async move {
        axum::serve(socket, router).await.unwrap();
    });
    (url, task)
}

struct Fixture {
    _directory: tempfile::TempDir,
    app: Arc<App>,
    backend: Arc<Upstream>,
    url: String,
    hooks: String,
    tasks: Vec<tokio::task::JoinHandle<()>>,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        for task in &self.tasks {
            task.abort();
        }
    }
}

impl Fixture {
    async fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let backend = Arc::new(Upstream::default());
        *backend.metrics.lock().await =
            "# TYPE frp_server_client_counts gauge\nfrp_server_client_counts 0\n".into();
        let upstream = Router::new()
            .route("/register", post(acme_register))
            .route(
                "/api/v1/servers/localhost/zones/homes.test.",
                get(|| async { Json(json!({})) }).patch(dns_patch),
            )
            .route(
                "/metrics",
                get(|State(state): State<Arc<Upstream>>| async move {
                    state.metrics.lock().await.clone()
                }),
            )
            .route(
                "/update",
                post(|| async { Json(json!({"txt":"accepted"})) }),
            )
            .with_state(backend.clone());
        let (upstream_url, task) = serve(upstream).await;
        let config = Config {
            listen: "127.0.0.1:0".into(),
            hook_listen: "127.0.0.1:0".into(),
            state: directory.path().join("state.json"),
            zone: "homes.test".into(),
            relay_ip: "192.0.2.10".parse().unwrap(),
            relay: Relay {
                host: "relay.homes.test".into(),
                port: 443,
                server_name: "relay.homes.test".into(),
            },
            acme_url: "https://names.homes.test/acme".into(),
            acme_internal: upstream_url.clone(),
            pdns_url: upstream_url.clone(),
            pdns_key: "test-key".into(),
            issuer: "pebble.test".into(),
            metrics_url: format!("{upstream_url}/metrics"),
            metrics_user: "metrics".into(),
            metrics_password: "test-password".into(),
        };
        let store = Store::open(config.state.clone()).unwrap();
        let app = App::new(config, store).unwrap();
        app.poll_metrics().await.unwrap();
        let (url, public) = serve(app.router()).await;
        let (hooks, hook_task) = serve(app.hooks()).await;
        Self {
            _directory: directory,
            app,
            backend,
            url,
            hooks,
            tasks: vec![task, public, hook_task],
        }
    }

    async fn invite(&self) -> String {
        admin::execute(&self.app, Command::Invite { valid_seconds: 300 })
            .await
            .unwrap()["invite"]
            .as_str()
            .unwrap()
            .into()
    }

    fn signed(
        &self,
        key: &SecretKey,
        method: &str,
        path: &str,
        body: &str,
        nonce: &str,
        timestamp: u64,
    ) -> reqwest::RequestBuilder {
        let timestamp = timestamp.to_string();
        let signature =
            key.sign(canonical(method, path, &timestamp, nonce, body.as_bytes()).as_bytes());
        self.app
            .client
            .request(method.parse().unwrap(), format!("{}{path}", self.url))
            .header("X-Wonton-Endpoint", key.public().to_string())
            .header("X-Wonton-Timestamp", timestamp)
            .header("X-Wonton-Nonce", nonce)
            .header("X-Wonton-Signature", hex(&signature.to_bytes()))
            .header("Content-Type", "application/json")
            .body(body.to_owned())
    }

    async fn request(
        &self,
        key: &SecretKey,
        method: &str,
        path: &str,
        value: Value,
    ) -> reqwest::Response {
        let body = if method == "GET" || method == "DELETE" {
            String::new()
        } else {
            value.to_string()
        };
        self.signed(
            key,
            method,
            path,
            &body,
            &random_token().unwrap()[..32],
            now(),
        )
        .send()
        .await
        .unwrap()
    }

    async fn register(&self, key: &SecretKey, invite: &str) -> Value {
        let response = self
            .request(key, "POST", "/homes", json!({"invite":invite}))
            .await;
        assert_eq!(response.status(), 201);
        response.json().await.unwrap()
    }

    async fn hook(&self, op: &str, home: &str, token: &str, extra: Value) -> Value {
        let mut content = if op == "Login" {
            json!({"user":home,"metas":{"token":token}})
        } else {
            json!({"user":{"user":home,"metas":{"token":token}}})
        };
        for (key, value) in extra.as_object().unwrap() {
            content[key] = value.clone();
        }
        self.app
            .client
            .post(format!("{}/frp", self.hooks))
            .json(&json!({"version":"0.1.0","op":op,"content":content}))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap()
    }
}

fn key(byte: u8) -> SecretKey {
    SecretKey::from_bytes(&[byte; 32])
}

#[tokio::test]
async fn registration_recovers_response_without_reusing_invite_or_label() {
    let fixture = Fixture::new().await;
    let invite = fixture.invite().await;
    let first = fixture.register(&key(1), &invite).await;
    let label = first["home"].as_str().unwrap();
    assert_eq!(label.len(), 8);
    assert!(
        label
            .bytes()
            .all(|b| b.is_ascii_lowercase() || (b'2'..=b'7').contains(&b))
    );
    assert_eq!(first["domain"], format!("{label}.homes.test"));
    assert_eq!(first["acme_dns_url"], "https://names.homes.test/acme");
    let second = fixture
        .request(&key(1), "POST", "/homes", json!({"invite":invite}))
        .await;
    assert_eq!(second.status(), 200);
    let second: Value = second.json().await.unwrap();
    assert_eq!(first["home"], second["home"]);
    assert_ne!(first["frp_token"], second["frp_token"]);
    assert_eq!(
        fixture
            .request(&key(2), "POST", "/homes", json!({"invite":invite}))
            .await
            .status(),
        403
    );
    assert_eq!(fixture.backend.next.load(Ordering::SeqCst), 2);
    assert_eq!(
        fixture
            .hook(
                "Login",
                label,
                first["frp_token"].as_str().unwrap(),
                json!({})
            )
            .await["reject"],
        true
    );
    assert_eq!(
        fixture
            .hook(
                "Login",
                label,
                second["frp_token"].as_str().unwrap(),
                json!({})
            )
            .await["reject"],
        false
    );
    assert_eq!(
        fixture
            .request(&key(1), "DELETE", &format!("/homes/{label}"), json!({}))
            .await
            .status(),
        200
    );
    assert_eq!(
        fixture
            .request(&key(1), "POST", "/homes", json!({"invite":invite}))
            .await
            .status(),
        403
    );
    assert!(fixture.app.store.lock().await.data.homes[label].deleted);
    let latest = fixture.backend.patches.lock().await.last().unwrap().clone();
    assert!(
        latest["rrsets"]
            .as_array()
            .unwrap()
            .iter()
            .all(|rr| rr["changetype"] == "DELETE")
    );
}

#[tokio::test]
async fn signature_checks_raw_body_timestamp_and_durable_nonce() {
    let fixture = Fixture::new().await;
    let invite = fixture.invite().await;
    let body = format!("{{ \"invite\" : \"{invite}\" }}");
    let nonce = "00000000000000000000000000000001";
    assert_eq!(
        fixture
            .signed(&key(1), "POST", "/homes", &body, nonce, now())
            .send()
            .await
            .unwrap()
            .status(),
        201
    );
    assert_eq!(
        fixture
            .signed(&key(1), "POST", "/homes", &body, nonce, now())
            .send()
            .await
            .unwrap()
            .status(),
        401
    );
    let altered = fixture
        .signed(
            &key(2),
            "POST",
            "/homes",
            &body,
            "00000000000000000000000000000002",
            now(),
        )
        .body(body.replace(' ', ""));
    assert_eq!(altered.send().await.unwrap().status(), 401);
    assert_eq!(
        fixture
            .signed(
                &key(2),
                "POST",
                "/homes",
                &body,
                "00000000000000000000000000000003",
                now() - 301
            )
            .send()
            .await
            .unwrap()
            .status(),
        401
    );
    let persisted: Value =
        serde_json::from_slice(&std::fs::read(&fixture.app.config.state).unwrap()).unwrap();
    assert_eq!(persisted["nonces"].as_object().unwrap().len(), 1);
}

#[tokio::test]
async fn invite_consumption_is_atomic_across_concurrent_endpoints() {
    let fixture = Fixture::new().await;
    let invite = fixture.invite().await;
    let a = key(1);
    let b = key(2);
    let (left, right) = tokio::join!(
        fixture.request(&a, "POST", "/homes", json!({"invite":invite})),
        fixture.request(&b, "POST", "/homes", json!({"invite":invite}))
    );
    let statuses = [left.status().as_u16(), right.status().as_u16()];
    assert!(statuses == [201, 403] || statuses == [403, 201]);
    assert_eq!(fixture.app.store.lock().await.data.homes.len(), 1);
}

#[tokio::test]
async fn suspension_removes_dns_and_rejects_all_admission_hooks() {
    let fixture = Fixture::new().await;
    let response = fixture.register(&key(1), &fixture.invite().await).await;
    let home = response["home"].as_str().unwrap();
    let token = response["frp_token"].as_str().unwrap();
    let proxy = json!({"proxy_type":"https","proxy_name":format!("{home}.matrix"),"custom_domains":[format!("{home}.homes.test")]});
    let admitted = fixture.hook("NewProxy", home, token, proxy.clone()).await;
    assert_eq!(admitted["reject"], false);
    assert_eq!(admitted["content"]["bandwidth_limit"], "5MB");
    assert_eq!(admitted["content"]["bandwidth_limit_mode"], "server");
    admin::execute(
        &fixture.app,
        Command::Suspend {
            home: home.into(),
            reason: "complaint".into(),
        },
    )
    .await
    .unwrap();
    for op in ["Login", "NewProxy", "NewUserConn", "Ping"] {
        assert_eq!(
            fixture.hook(op, home, token, proxy.clone()).await["reject"],
            true,
            "{op}"
        );
    }
    let status: Value = fixture
        .request(&key(1), "GET", &format!("/homes/{home}"), json!({}))
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(status["suspended"], true);
    assert_eq!(status["reason"], "complaint");
    admin::execute(&fixture.app, Command::Resume { home: home.into() })
        .await
        .unwrap();
    assert_eq!(
        fixture.hook("Login", home, token, json!({})).await["reject"],
        false
    );
}

#[tokio::test]
async fn caa_first_binding_is_effective_and_changes_have_stable_cooldown() {
    let fixture = Fixture::new().await;
    let response = fixture.register(&key(1), &fixture.invite().await).await;
    let home = response["home"].as_str().unwrap();
    let path = format!("/homes/{home}/caa");
    assert_eq!(
        fixture
            .request(
                &key(1),
                "PUT",
                &path,
                json!({"account_uri":"https://pebble.test/acct/1"})
            )
            .await
            .status(),
        200
    );
    let caa = fixture.backend.patches.lock().await.last().unwrap().clone();
    assert!(caa["rrsets"].as_array().unwrap().iter().any(|rr| {
        rr["type"] == "CAA"
            && rr["records"][0]["content"]
                .as_str()
                .unwrap()
                .contains("accounturi=https://pebble.test/acct/1")
    }));
    let second = fixture
        .request(
            &key(1),
            "PUT",
            &path,
            json!({"account_uri":"https://pebble.test/acct/2"}),
        )
        .await;
    assert_eq!(second.status(), 202);
    let pending: Value = second.json().await.unwrap();
    let repeated: Value = fixture
        .request(
            &key(1),
            "PUT",
            &path,
            json!({"account_uri":"https://pebble.test/acct/2"}),
        )
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(pending["pending"], repeated["pending"]);
    fixture
        .app
        .store
        .lock()
        .await
        .data
        .homes
        .get_mut(home)
        .unwrap()
        .pending_caa
        .as_mut()
        .unwrap()
        .effective_at = now() - 1;
    fixture.app.reconcile().await.unwrap();
    assert_eq!(
        fixture.app.store.lock().await.data.homes[home]
            .caa
            .as_ref()
            .unwrap()
            .account_uri,
        "https://pebble.test/acct/2"
    );
}

#[tokio::test]
async fn proxy_ownership_and_entry_quota_cannot_be_bypassed() {
    let fixture = Fixture::new().await;
    let response = fixture.register(&key(1), &fixture.invite().await).await;
    let home = response["home"].as_str().unwrap();
    let token = response["frp_token"].as_str().unwrap();
    for domain in [
        format!("*.{home}.homes.test"),
        format!("lan.{home}.homes.test"),
        format!("x.lan.{home}.homes.test"),
        "another.homes.test".into(),
    ] {
        assert_eq!(fixture.hook("NewProxy",home,token,json!({"proxy_type":"https","proxy_name":format!("{home}.bad"),"custom_domains":[domain]})).await["reject"],true);
    }
    for n in 0..5 {
        assert_eq!(fixture.hook("NewProxy",home,token,json!({"proxy_type":"https","proxy_name":format!("{home}.p{n}"),"custom_domains":[format!("p{n}.{home}.homes.test")]})).await["reject"],false);
    }
    assert_eq!(fixture.hook("NewProxy",home,token,json!({"proxy_type":"https","proxy_name":format!("{home}.sixth"),"custom_domains":[format!("sixth.{home}.homes.test")]})).await["reject"],true);
    assert_eq!(
        fixture
            .hook(
                "CloseProxy",
                home,
                token,
                json!({"proxy_name":"another.matrix"})
            )
            .await["reject"],
        true
    );
    assert_eq!(
        fixture
            .hook(
                "CloseProxy",
                home,
                token,
                json!({"proxy_name":format!("{home}.p0")})
            )
            .await["reject"],
        false
    );
    assert_eq!(fixture.hook("NewProxy",home,token,json!({"proxy_type":"https","proxy_name":format!("{home}.sixth"),"custom_domains":[format!("sixth.{home}.homes.test")]})).await["reject"],false);
    fixture
        .app
        .runtime
        .lock()
        .await
        .connections
        .insert(home.into(), 100);
    assert_eq!(
        fixture.hook("NewUserConn", home, token, json!({})).await["reject"],
        true
    );
    fixture
        .app
        .store
        .lock()
        .await
        .data
        .homes
        .get_mut(home)
        .unwrap()
        .usage_bytes = 100_000_000_000;
    assert_eq!(
        fixture.hook("Ping", home, token, json!({})).await["reject"],
        true
    );
}

#[test]
fn state_lock_prevents_two_writers_and_preserves_invites_after_restart() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("state.json");
    let mut store = Store::open(path.clone()).unwrap();
    store.data.invites = BTreeMap::from([("digest".into(), 123)]);
    store.save().unwrap();
    assert!(Store::open(path.clone()).is_err());
    drop(store);
    assert_eq!(Store::open(path).unwrap().data.invites["digest"], 123);
}

#[test]
fn quota_month_uses_calendar_utc_not_fixed_thirty_day_windows() {
    assert_eq!(crate::metrics::utc_month(0), 1970 * 12);
    assert_eq!(crate::metrics::utc_month(2678399), 1970 * 12);
    assert_eq!(crate::metrics::utc_month(2678400), 1970 * 12 + 1);
}
