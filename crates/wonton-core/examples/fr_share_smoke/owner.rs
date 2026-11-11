use anyhow::{Context, Result, ensure};
use iroh::{Endpoint, EndpointAddr, RelayMode, RelayUrl, endpoint::presets};
use reqwest::{Client, Method};
use serde_json::{Value, json};
use std::{path::PathBuf, time::Duration};
use wonton_tunnel::{LocalProxy, ProxyAccess, TunnelClient};

pub struct Owner {
    pub http: Client,
    pub domain: String,
    pub endpoint: Endpoint,
    proxy: LocalProxy,
    tunnel: TunnelClient,
}

pub fn state() -> Result<PathBuf> {
    Ok(PathBuf::from(std::env::var("SHARE_SMOKE_STATE")?))
}

pub fn https() -> Result<Client> {
    let pem = std::fs::read(state()?.join("ca.pem"))?;
    Ok(Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(20))
        .add_root_certificate(reqwest::Certificate::from_pem(&pem)?)
        .build()?)
}

pub async fn endpoint() -> Result<Endpoint> {
    let relay: RelayUrl = "https://relay.test/".parse()?;
    // iroh 只信编译进去的根；测试中继用 Pebble 签的证书，显式加一份。
    let pem = std::fs::read(state()?.join("ca.pem"))?;
    let roots =
        rustls_pemfile::certs(&mut std::io::Cursor::new(pem)).collect::<Result<Vec<_>, _>>()?;
    Ok(Endpoint::builder(presets::Minimal)
        .relay_mode(RelayMode::Custom(iroh::RelayMap::from_iter([relay])))
        .ca_tls_config(iroh::tls::CaTlsConfig::custom_roots(roots))
        .bind()
        .await?)
}

impl Owner {
    pub async fn open(server: &str) -> Result<Self> {
        let domain = format!("{server}.test");
        let direct = Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(20))
            .build()?;
        let base = format!("http://core-{server}:8080");
        let token = std::fs::read_to_string(state()?.join(format!("{server}/core/owner-token")))?;
        // 直连容器也要带域名 Host，网关按它选路由。
        let host = format!("{domain}:8080");
        // 核心的健康检查只看 HTTP；隧道要等它连上中继，502 时重试一会儿。
        let mut attempt = 0;
        let info: Value = loop {
            let reply = direct
                .get(format!("{base}/_wonton/api/tunnel"))
                .header("Host", &host)
                .bearer_auth(token.trim())
                .send()
                .await?;
            if reply.status().is_success() {
                break reply.json().await?;
            }
            attempt += 1;
            ensure!(
                attempt < 30,
                "tunnel API never became ready: {}",
                reply.text().await?
            );
            tokio::time::sleep(Duration::from_secs(1)).await;
        };
        let endpoint = endpoint().await?;
        direct
            .post(format!("{base}/_wonton/api/devices"))
            .header("Host", &host)
            .bearer_auth(token.trim())
            .json(&json!({"endpoint_id":endpoint.id().to_string()}))
            .send()
            .await?
            .error_for_status()?;
        // 直连地址来自隔离网络的 DNS，中继只作兜底：冒烟不依赖中继的数据面。
        let direct: std::net::SocketAddr =
            std::net::ToSocketAddrs::to_socket_addrs(&(format!("core-{server}"), 8082u16))?
                .next()
                .context("core tunnel address")?;
        let target = EndpointAddr::new(
            info["endpoint_id"]
                .as_str()
                .context("endpoint id")?
                .parse()?,
        )
        .with_ip_addr(direct)
        .with_relay_url("https://relay.test/".parse()?);
        let tunnel = TunnelClient::new(endpoint.clone(), target);
        let proxy =
            LocalProxy::bind(tunnel.clone(), "127.0.0.1:0".parse()?, ProxyAccess::Open).await?;
        let http = Client::builder()
            .proxy(reqwest::Proxy::http(format!(
                "http://{}",
                proxy.local_addr()
            ))?)
            .timeout(Duration::from_secs(60))
            .build()?;
        Ok(Self {
            http,
            domain,
            endpoint,
            proxy,
            tunnel,
        })
    }

    pub async fn api(&self, method: Method, path: &str, body: Option<Value>) -> Result<Value> {
        let mut request = self
            .http
            .request(method, format!("http://{}:8080{path}", self.domain));
        if let Some(body) = body {
            request = request.json(&body);
        }
        let response = request.send().await?;
        let status = response.status();
        let bytes = response.bytes().await?;
        ensure!(
            status.is_success(),
            "owner API {path} returned {status}: {}",
            String::from_utf8_lossy(&bytes)
        );
        if bytes.is_empty() {
            return Ok(Value::Null);
        }
        Ok(serde_json::from_slice(&bytes)?)
    }

    pub async fn close(self) {
        self.proxy.shutdown().await;
        self.tunnel.close();
        self.endpoint.close().await;
    }
}

pub async fn pair(a: &Owner, b: &Owner) -> Result<()> {
    let code = a.api(Method::GET, "/_wonton/api/peers/code", None).await?;
    b.api(
        Method::POST,
        "/_wonton/api/peers",
        Some(json!({"code":code["code"]})),
    )
    .await?;
    let view = a.api(Method::GET, "/_wonton/api/peers", None).await?;
    let id = view["requests"]
        .as_array()
        .context("peer requests")?
        .first()
        .and_then(|request| request["id"].as_str())
        .context("incoming peer request")?;
    a.api(
        Method::POST,
        &format!("/_wonton/api/peers/requests/{id}/approve"),
        None,
    )
    .await?;
    let view = a.api(Method::GET, "/_wonton/api/peers", None).await?;
    ensure!(
        view["peers"]
            .as_array()
            .context("paired peers")?
            .iter()
            .any(|peer| peer["server_name"] == "friend.test"),
        "peer approval did not persist"
    );
    Ok(())
}
