use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::extract::{OriginalUri, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, post};
use base64::Engine as _;
use rmcp::model::{ClientCapabilities, ClientConfig, Implementation};
use rmcp::service::{RoleClient, RunningService};
use rmcp::transport::StreamableHttpClientTransport;
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use rmcp::{ClientLifecycleMode, ClientServiceExt};
use serde_json::{Value, json};

use crate::plugin::fake_docker::FakeDocker;
use crate::plugin::names::container_name;
use crate::plugin::storefront::StoreConfig;
use crate::plugin::tests::{langs, manager::manager_with};
use crate::plugin::{InstallError, PluginId, PluginManager, PluginPackage};

pub(crate) const REMOTE: &str = "com.example.remote-tools";
pub(crate) const PROVIDER: &str = "com.example.connector";
pub(crate) const TEMPLATE: &str = "https://mcp.example.com/mcp/{secret:token}?key={secret:token}";
pub(crate) const SECRET: &str = "old/one?x=1";

type ReceivedRequests = Arc<Mutex<Vec<(String, HeaderMap)>>>;

pub(crate) fn id(value: &str) -> PluginId {
    PluginId::parse(value).unwrap()
}

pub(crate) fn remote(auth: &str) -> PluginPackage {
    let mut doc = json!({"x-wonton": {
        "spec": "0.15", "id": REMOTE, "version": "1.0.0", "kind": "app",
        "runtime": "remote_mcp", "title": {"en": "Remote tools"},
        "description": {"en": "Remote tools"}, "permissions": [],
        "remote_mcp": {"url": TEMPLATE, "auth": auth, "tools": ["search"],
            "tool_idle_seconds": 47, "tool_total_seconds": 91},
        "secrets": {"token": {"input": {"title": {"en": "Token"}}}}
    }});
    if auth == "header" {
        doc["x-wonton"]["remote_mcp"]["headers"] =
            json!({"Authorization": "Bearer {secret:token}"});
    } else if auth == "oauth" {
        doc["x-wonton"]["remote_mcp"]["url"] = json!("https://mcp.example.com/mcp");
        doc["x-wonton"].as_object_mut().unwrap().remove("secrets");
    }
    PluginPackage::parse(&doc.to_string(), &langs()).unwrap()
}

fn provider(port: u16) -> PluginPackage {
    PluginPackage::parse(
        &json!({
            "services": {"connector": {"image": "test-only"}},
            "x-wonton": {"spec": "0.15", "id": PROVIDER, "version": "1.0.0", "kind": "app",
                "title": {"en": "Connector"}, "description": {"en": "Connector"}, "permissions": [],
                "provides": {"remote_mcp": {"service": "connector", "port": port}}}
        })
        .to_string(),
        &langs(),
    )
    .unwrap()
}

#[derive(Clone)]
pub(crate) struct Fake {
    pub outcome: Arc<Mutex<String>>,
    pub probes: Arc<Mutex<Vec<Value>>>,
    pub forwarded: Arc<Mutex<Vec<(String, HeaderMap)>>>,
    pub calls: Arc<Mutex<Vec<(String, HeaderMap)>>>,
    pub forgotten: Arc<Mutex<Vec<String>>>,
    pub forget_status: Arc<Mutex<StatusCode>>,
    remote: String,
    http: reqwest::Client,
}

struct Server(tokio::task::JoinHandle<()>);
impl Drop for Server {
    fn drop(&mut self) {
        self.0.abort();
    }
}

async fn serve(app: axum::Router) -> (String, Server) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (addr.to_string(), Server(task))
}

async fn probe(State(state): State<Fake>, axum::Json(body): axum::Json<Value>) -> Response {
    state.probes.lock().unwrap().push(body.clone());
    let outcome = state.outcome.lock().unwrap().clone();
    if outcome == "http500" {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }
    if outcome == "malformed" {
        return axum::Json(json!({"outcome": 3})).into_response();
    }
    axum::Json(json!({"outcome": outcome, "http_status": 401,
        "detail": format!("remote URL {} header {}", body["url"].as_str().unwrap(),
            body["headers"]["authorization"].as_str().unwrap_or_default())}))
    .into_response()
}

async fn forget(State(state): State<Fake>, OriginalUri(uri): OriginalUri) -> StatusCode {
    state.forgotten.lock().unwrap().push(uri.path().to_owned());
    *state.forget_status.lock().unwrap()
}

async fn forward(State(state): State<Fake>, headers: HeaderMap, body: String) -> Response {
    assert_eq!(headers["x-wonton-plugin"], REMOTE);
    let url = headers["x-wonton-remote-url"].to_str().unwrap().to_owned();
    state
        .forwarded
        .lock()
        .unwrap()
        .push((url.clone(), headers.clone()));
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(headers["x-wonton-remote-forward"].as_bytes())
        .unwrap();
    let forwarded: BTreeMap<String, String> = serde_json::from_slice(&bytes).unwrap();
    let auth = headers["x-wonton-remote-auth"].to_str().unwrap();
    assert!(matches!(auth, "none" | "header" | "oauth"));
    assert!(auth == "header" || forwarded.is_empty());
    let url = url::Url::parse(&url).unwrap();
    let mut outgoing = state.http.post(format!(
        "http://{}{}{}",
        state.remote,
        url.path(),
        url.query().map_or_else(String::new, |q| format!("?{q}"))
    ));
    for name in [
        "content-type",
        "accept",
        "mcp-session-id",
        "mcp-protocol-version",
    ] {
        if let Some(value) = headers.get(name) {
            outgoing = outgoing.header(name, value);
        }
    }
    for (name, value) in forwarded {
        outgoing = outgoing.header(name, value);
    }
    let response = outgoing.body(body).send().await.unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let mut output = (status, response.bytes().await.unwrap()).into_response();
    *output.headers_mut() = headers;
    output
}

async fn remote_rpc(
    State(calls): State<ReceivedRequests>,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
    axum::Json(body): axum::Json<Value>,
) -> Response {
    calls.lock().unwrap().push((uri.to_string(), headers));
    let result = match body["method"].as_str().unwrap() {
        "initialize" => json!({"protocolVersion": "2025-11-25", "capabilities": {"tools": {}},
            "serverInfo": {"name": "fake-remote", "version": "1"}}),
        "tools/list" => json!({"tools": [{"name": "search", "inputSchema": {"type": "object"},
            "annotations": {"readOnlyHint": true}},
            {"name": "hidden", "inputSchema": {"type": "object"}}]}),
        "tools/call" => json!({"content": [{"type": "text", "text": "remote-called"}]}),
        _ => return StatusCode::ACCEPTED.into_response(),
    };
    axum::Json(json!({"jsonrpc": "2.0", "id": body["id"], "result": result})).into_response()
}

pub(crate) struct Bed {
    pub manager: Arc<PluginManager>,
    pub docker: FakeDocker,
    pub fake: Fake,
    pub data: tempfile::TempDir,
    servers: Vec<Server>,
    index: tempfile::TempDir,
    provider: PluginPackage,
}

impl Bed {
    pub(crate) async fn new(with_provider: bool) -> Self {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let (remote, remote_server) = serve(
            axum::Router::new()
                .fallback(remote_rpc)
                .with_state(calls.clone()),
        )
        .await;
        let fake = Fake {
            outcome: Arc::new(Mutex::new("ok".into())),
            probes: Arc::default(),
            forwarded: Arc::default(),
            calls,
            forgotten: Arc::default(),
            forget_status: Arc::new(Mutex::new(StatusCode::NO_CONTENT)),
            remote,
            http: reqwest::Client::builder().no_proxy().build().unwrap(),
        };
        let (connector, connector_server) = serve(
            axum::Router::new()
                .route("/_wonton/probe", post(probe))
                .route("/_wonton/plugins/{id}", delete(forget))
                .route("/mcp", post(forward))
                .with_state(fake.clone()),
        )
        .await;
        let provider = provider(connector.parse::<std::net::SocketAddr>().unwrap().port());
        let (docker, engine) = FakeDocker::start().await;
        let data = tempfile::tempdir().unwrap();
        let index = tempfile::tempdir().unwrap();
        std::fs::write(
            index.path().join("index.json"),
            "{\"format\":1,\"plugins\":[]}",
        )
        .unwrap();
        let manager = Arc::new(manager_with(engine, &data).with_store(StoreConfig {
            index_dir: Some(index.path().to_path_buf()),
            community: false,
        }));
        if with_provider {
            manager
                .install_package(provider.clone(), &[], langs())
                .await
                .unwrap();
        }
        manager.secrets_for_test(&id(REMOTE), &[("token", SECRET)]);
        Self {
            manager,
            docker,
            fake,
            data,
            servers: vec![remote_server, connector_server],
            index,
            provider,
        }
    }

    pub(crate) async fn install(&self, auth: &str) -> Result<(), InstallError> {
        self.manager
            .install_package(remote(auth), &[], langs())
            .await
            .map(|_| ())
    }

    pub(crate) fn publish_provider(&self) {
        crate::plugin::storefront::index::IndexEntry::of(
            &self.provider,
            "github:example/connector",
            &"c".repeat(40),
        )
        .unwrap()
        .write_to(self.index.path(), &self.provider)
        .unwrap();
    }

    pub(crate) fn provider_container(&self) -> String {
        container_name(&id(PROVIDER).slug(), "connector")
    }

    pub(crate) async fn agent(&mut self) -> RunningService<RoleClient, ClientConfig> {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        self.manager.register_source_for_test(
            "com.example.agent",
            "127.0.0.0/8",
            vec!["tools".parse().unwrap()],
        );
        let confirmations = Arc::new(crate::mcp::Confirmations::new(
            Arc::new(crate::mcp::test_support::Cards::default()),
            Duration::from_secs(60),
        ));
        let bus = crate::mcp::Bus::new(
            self.manager.clone(),
            confirmations,
            Arc::new(crate::mcp::audit::Audit::new(
                self.data.path().join("audit.jsonl"),
            )),
        );
        let (tunnel, members) = crate::gateway::fakes::loopback_tunnel(self.data.path()).await;
        let gateway = crate::gateway::Gateway::new(
            self.manager.cfg.public.clone(),
            self.manager.cfg.profile.clone(),
            "contract-owner".into(),
            members,
            self.manager.clone(),
            None,
            crate::http::HttpClient::new(),
            tunnel,
        )
        .with_mcp(Some(bus.endpoint()));
        self.servers.push(Server(tokio::spawn(async move {
            axum::serve(
                listener,
                crate::gateway::router(Arc::new(gateway))
                    .into_make_service_with_connect_info::<std::net::SocketAddr>(),
            )
            .await
            .unwrap();
        })));
        let http = reqwest::Client::builder()
            .no_proxy()
            .resolve("mcp.wonton.internal", addr)
            .build()
            .unwrap();
        let transport = StreamableHttpClientTransport::with_client(
            http,
            StreamableHttpClientTransportConfig::with_uri(format!(
                "http://mcp.wonton.internal:{}/mcp",
                addr.port()
            )),
        );
        ClientConfig::new(
            ClientCapabilities::default(),
            Implementation::new("contract-agent", "1"),
        )
        .serve_with_lifecycle(transport, ClientLifecycleMode::Initialize)
        .await
        .unwrap()
    }
}
