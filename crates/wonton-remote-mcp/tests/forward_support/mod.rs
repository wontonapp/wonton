use std::net::SocketAddr;

use axum::{Router, http::HeaderMap};
use base64::Engine;
use wonton_remote_mcp::{AddressPolicy, Config};

pub struct Server {
    pub address: SocketAddr,
    task: tokio::task::JoinHandle<()>,
}

impl Server {
    pub fn url(&self) -> String {
        format!("http://{}", self.address)
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}

pub async fn spawn(router: Router) -> Server {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    Server { address, task }
}

pub async fn connector() -> Server {
    spawn(wonton_remote_mcp::router(Config {
        address_policy: AddressPolicy::AllowPrivate,
        ..Config::default()
    }))
    .await
}

pub fn headers(url: &str, credentials: serde_json::Value) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert("x-wonton-plugin", "app.example.tools".parse().unwrap());
    headers.insert("x-wonton-remote-url", url.parse().unwrap());
    headers.insert(
        "x-wonton-remote-auth",
        if credentials.as_object().unwrap().is_empty() {
            "none"
        } else {
            "header"
        }
        .parse()
        .unwrap(),
    );
    headers.insert(
        "x-wonton-remote-forward",
        base64::engine::general_purpose::STANDARD
            .encode(serde_json::to_vec(&credentials).unwrap())
            .parse()
            .unwrap(),
    );
    headers.insert(
        "accept",
        "application/json, text/event-stream".parse().unwrap(),
    );
    headers.insert("content-type", "application/json".parse().unwrap());
    headers
}

pub fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .no_proxy()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .unwrap()
}

pub async fn sdk_contract(connector: &Server, headers: HeaderMap) {
    use rmcp::{
        ClientLifecycleMode, ClientServiceExt,
        model::{CallToolRequestParams, ClientCapabilities, ClientConfig, Implementation},
        transport::{
            StreamableHttpClientTransport,
            streamable_http_client::StreamableHttpClientTransportConfig,
        },
    };
    let mut config =
        StreamableHttpClientTransportConfig::with_uri(format!("{}/mcp", connector.url()));
    config.custom_headers = headers
        .into_iter()
        .filter_map(|(name, value)| {
            name.filter(|name| name != "accept" && name != "content-type")
                .map(|name| (name, value))
        })
        .collect();
    let transport = StreamableHttpClientTransport::with_client(client(), config);
    let service = ClientConfig::new(
        ClientCapabilities::default(),
        Implementation::new("remote-mcp-contract", "1"),
    )
    .serve_with_lifecycle(transport, ClientLifecycleMode::Initialize)
    .await
    .unwrap();
    let tools = service.peer().list_all_tools().await.unwrap();
    assert_eq!(
        tools
            .iter()
            .map(|tool| tool.name.as_ref())
            .collect::<Vec<_>>(),
        ["echo"]
    );
    let result = service
        .peer()
        .call_tool(CallToolRequestParams::new("echo"))
        .await
        .unwrap();
    let result = serde_json::to_value(result).unwrap();
    assert_eq!(result["content"][0]["text"], "remote-result");
    service.cancel().await.unwrap();
}
