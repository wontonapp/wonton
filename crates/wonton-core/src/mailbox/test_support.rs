use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::extract::{Path, State};
use axum::routing::get;
use axum::{Json, Router};
use serde_json::{Value, json};

use crate::gateway::Gateway;
use crate::http::HttpClient;
use crate::matrix::{Matrix, NoticeRoom};
use crate::members::MemberRecord;
use crate::owner::{Owner, OwnerStore};
use crate::plugin::docker::Engine;
use crate::plugin::image_worker::ImageWorker;
use crate::plugin::languages::Languages;
use crate::plugin::lifecycle::LifecycleConfig;
use crate::plugin::store::Store;
use crate::plugin::{Permission, PluginManager, PluginPackage, PluginStatus};

pub(super) const PLUGIN: &str = "app.test.mail";
pub(super) const PASSWORD: &str = "test-mailpit-password";

pub(super) struct Home {
    pub(super) gateway: Arc<Gateway>,
    pub(super) matrix: crate::matrix::fake::Fake,
    pub(super) dir: tempfile::TempDir,
}

pub(super) async fn home() -> Home {
    let dir = tempfile::tempdir().unwrap();
    let (fake, config) = crate::matrix::fake::start().await;
    let matrix = Arc::new(
        Matrix::connect(&config, dir.path(), HttpClient::new())
            .await
            .unwrap(),
    );
    let public = crate::config::PublicUrls {
        scheme: "http".into(),
        base_domain: "wonton.test".into(),
        port: 8080,
    };
    let profile = OwnerStore::for_test(
        dir.path(),
        &Owner {
            username: "owner".into(),
            display_name: "Owner".into(),
            email: None,
        },
    );
    let cfg = LifecycleConfig {
        public: public.clone(),
        data_dir: dir.path().to_owned(),
        apps_dir: dir.path().join("apps"),
        shares_dir: dir.path().join("shares"),
        core_container: None,
        tz: "UTC".into(),
        puid: 1000,
        pgid: 1000,
        install_timeout: Duration::from_secs(5),
        owner: "owner".into(),
        profile: profile.clone(),
        gateway_port: 8080,
        plugin_slice: None,
        instance: Default::default(),
    };
    let plugins = Arc::new(PluginManager::new(
        Engine::unreachable_for_test(),
        Store::open(dir.path().join("plugins")).unwrap(),
        HttpClient::new(),
        None,
        cfg,
        ImageWorker::in_process(),
    ));
    let package = PluginPackage::parse(&format!(
        "services:\n  app:\n    image: example/app\nx-wonton:\n  spec: '0.11'\n  id: {PLUGIN}\n  version: 0.1.0\n  kind: app\n  title: {{en: Mail app}}\n  description: {{en: Mail app}}\n  permissions: [notify]\n"
    ), &Languages::default()).unwrap();
    plugins.register_for_test(package.manifest, PluginStatus::Running, None);
    plugins.register_source_for_test(PLUGIN, "172.30.1.0/24", vec![Permission::Notify]);
    let (tunnel, members) = crate::gateway::fakes::loopback_tunnel(dir.path()).await;
    members
        .invite(MemberRecord {
            username: "alice".into(),
            user_id: "@alice:test".into(),
            display_name: "Alice".into(),
            email: None,
            status: wonton_protocol::MemberStatus::Invited,
            can_install_plugins: false,
            wants_private_bot: false,
            private_bot: None,
            pairing_code_hash: None,
            pairing_code_expires_at_ms: None,
        })
        .unwrap();
    members.consume_code("alice").unwrap();
    let notices = Arc::new(NoticeRoom::new(
        matrix.clone(),
        "owner",
        dir.path().join("notice-room"),
        Arc::new(Languages::default),
    ));
    let gateway = Arc::new(
        Gateway::new(
            public,
            profile,
            "owner-test-token".into(),
            members,
            plugins,
            None,
            HttpClient::new(),
            tunnel,
        )
        .with_notices(Some(notices))
        .with_matrix(Some(matrix)),
    );
    Home {
        gateway,
        matrix: fake,
        dir,
    }
}

#[derive(Default)]
pub(super) struct StateData {
    pub(super) messages: std::collections::BTreeMap<String, Value>,
    pub(super) received: std::collections::BTreeMap<String, Value>,
    pub(super) deleted: Vec<String>,
    pub(super) fail_delete: bool,
    pub(super) parts: std::collections::BTreeMap<(String, String), Vec<u8>>,
    pub(super) connections: usize,
    pub(super) closed: usize,
    pub(super) fail_list: bool,
}

pub(super) struct Mailpit {
    pub(super) authority: String,
    pub(super) state: Arc<Mutex<StateData>>,
    pub(super) events: tokio::sync::broadcast::Sender<Option<String>>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for Mailpit {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl Mailpit {
    pub(super) async fn start() -> Self {
        let state = Arc::new(Mutex::new(StateData::default()));
        let (events, _) = tokio::sync::broadcast::channel(8);
        let event_sender = events.clone();
        let app = Router::new()
            .route("/api/v1/messages", get(list).delete(delete))
            .route("/api/v1/message/{id}", get(message))
            .route("/api/v1/message/{id}/headers", get(headers))
            .route("/api/v1/message/{id}/part/{part}", get(attachment))
            .route(
                "/api/events",
                get(move |State(state), request| {
                    super::event_support::subscribe(state, event_sender.clone(), request)
                }),
            )
            .with_state(state.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let authority = listener.local_addr().unwrap().to_string();
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        Self {
            authority,
            state,
            events,
            task,
        }
    }

    pub(super) fn add(&self, id: &str, to: Value, received: Value) {
        let mut state = self.state.lock().unwrap();
        state.messages.insert(
            id.into(),
            json!({"ID":id,"From":{"Address":"arbitrary@example.com"},
            "Subject":"Password reset","Text":"Reset at https://example.com/reset","To":to,
            "Cc":null,"Bcc":null,"Attachments":null,"Inline":null}),
        );
        state.received.insert(id.into(), received);
    }

    pub(super) fn api(&self) -> super::api::Api {
        super::api::Api::new(&self.authority, PASSWORD, HttpClient::new()).unwrap()
    }
}

async fn list(
    State(state): State<Arc<Mutex<StateData>>>,
) -> Result<Json<Value>, hyper::StatusCode> {
    let state = state.lock().unwrap();
    if state.fail_list {
        return Err(hyper::StatusCode::SERVICE_UNAVAILABLE);
    }
    Ok(Json(
        json!({"total":state.messages.len(),"start":0,"messages":state.messages.keys().map(|id|json!({"ID":id})).collect::<Vec<_>>()}),
    ))
}
async fn message(
    State(state): State<Arc<Mutex<StateData>>>,
    Path(id): Path<String>,
) -> Json<Value> {
    Json(state.lock().unwrap().messages[&id].clone())
}
async fn headers(
    State(state): State<Arc<Mutex<StateData>>>,
    Path(id): Path<String>,
) -> Json<Value> {
    Json(state.lock().unwrap().received[&id].clone())
}
async fn attachment(
    State(state): State<Arc<Mutex<StateData>>>,
    Path(key): Path<(String, String)>,
) -> Result<Vec<u8>, hyper::StatusCode> {
    state
        .lock()
        .unwrap()
        .parts
        .get(&key)
        .cloned()
        .ok_or(hyper::StatusCode::NOT_FOUND)
}
async fn delete(
    State(state): State<Arc<Mutex<StateData>>>,
    Json(body): Json<Value>,
) -> hyper::StatusCode {
    let mut state = state.lock().unwrap();
    if state.fail_delete {
        return hyper::StatusCode::SERVICE_UNAVAILABLE;
    }
    for id in body["IDs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|id| id.as_str().unwrap())
    {
        state.messages.remove(id);
        state.deleted.push(id.to_owned());
    }
    hyper::StatusCode::OK
}

pub(super) fn received(ip: &str) -> Value {
    json!({"Received":[format!("from app (unknown [{ip}]) by mailpit (Mailpit) with SMTP for <owner@wonton.internal>; date")]})
}
