//! 家名服务只管理名字、DNS 和中继凭据；请求用本机的 iroh 身份签名。

use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, bail, ensure};
use iroh::SecretKey;
use reqwest::{Client, Method};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::json;
use sha2::{Digest, Sha256};
use url::Url;

#[derive(Clone, Serialize, Deserialize)]
pub struct AcmeDnsCredentials {
    pub username: String,
    pub password: String,
    pub subdomain: String,
    pub fulldomain: String,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Relay {
    pub host: String,
    pub port: u16,
    pub server_name: String,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Registration {
    pub home: String,
    pub domain: String,
    pub frp_token: String,
    pub acme_dns: AcmeDnsCredentials,
    pub lan_acme_dns: AcmeDnsCredentials,
    pub acme_dns_url: String,
    pub relay: Relay,
    pub suspended: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct HomeStatus {
    pub home: String,
    pub domain: String,
    pub suspended: bool,
    pub reason: Option<String>,
}

#[derive(Clone)]
pub struct HomeService {
    url: Url,
    client: Client,
    key: SecretKey,
}

impl HomeService {
    pub fn new(url: &str, ca: Option<&Path>, key: SecretKey) -> anyhow::Result<Self> {
        let url = Url::parse(url).context("家名服务地址不是 URL")?;
        ensure!(
            url.scheme() == "https" && url.host_str().is_some(),
            "家名服务必须使用 HTTPS"
        );
        ensure!(
            url.username().is_empty()
                && url.password().is_none()
                && url.query().is_none()
                && url.fragment().is_none()
                && url.path() == "/",
            "家名服务地址必须是无凭据的 HTTPS origin"
        );
        let mut builder = Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(30))
            .redirect(reqwest::redirect::Policy::none());
        if let Some(ca) = ca {
            for certificate in reqwest::Certificate::from_pem_bundle(&std::fs::read(ca)?)? {
                builder = builder.add_root_certificate(certificate);
            }
        }
        Ok(Self {
            url,
            client: builder.build()?,
            key,
        })
    }

    pub async fn register(&self, invite: &str) -> anyhow::Result<Registration> {
        let registration: Registration = self
            .request(Method::POST, "/homes", Some(json!({"invite": invite})))
            .await?;
        ensure!(!registration.suspended, "家名服务已停用这个名字");
        crate::install::validate_server_name(&registration.domain)?;
        ensure!(
            registration.home.len() == 8
                && registration
                    .home
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || matches!(b, b'2'..=b'7')),
            "家名服务返回了非法家名"
        );
        ensure!(
            registration
                .domain
                .starts_with(&format!("{}.", registration.home)),
            "家名与服务器名不匹配"
        );
        ensure!(
            !registration.frp_token.is_empty() && registration.relay.port != 0,
            "家名服务缺少中继凭据"
        );
        crate::install::validate_server_name(&registration.relay.host)?;
        crate::install::validate_server_name(&registration.relay.server_name)?;
        let acme = Url::parse(&registration.acme_dns_url)?;
        ensure!(
            acme.origin() == self.url.origin()
                && acme.path() == "/acme"
                && acme.query().is_none()
                && acme.username().is_empty()
                && acme.password().is_none(),
            "acme-dns 地址必须属于注册服务"
        );
        Ok(registration)
    }

    pub async fn status(&self, home: &str) -> anyhow::Result<HomeStatus> {
        self.request(Method::GET, &format!("/homes/{home}"), None)
            .await
    }

    pub async fn bind_caa(&self, home: &str, account_uri: &str) -> anyhow::Result<()> {
        let value: serde_json::Value = self
            .request(
                Method::PUT,
                &format!("/homes/{home}/caa"),
                Some(json!({"account_uri": account_uri})),
            )
            .await?;
        if value.get("pending").and_then(serde_json::Value::as_bool) == Some(true) {
            bail!("CAA 账号变更仍在冷却期，证书尚未签发");
        }
        Ok(())
    }

    async fn request<T: DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
        body: Option<serde_json::Value>,
    ) -> anyhow::Result<T> {
        let body = body
            .map(|v| serde_json::to_vec(&v))
            .transpose()?
            .unwrap_or_default();
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)?
            .as_secs()
            .to_string();
        let nonce = crate::secret::random_token()[..32].to_owned();
        let digest = hex(Sha256::digest(&body).as_slice());
        let canonical = format!("wonton-home-v1\n{method}\n{path}\n{timestamp}\n{nonce}\n{digest}");
        let signature = hex(&self.key.sign(canonical.as_bytes()).to_bytes());
        let response = self
            .client
            .request(method, self.url.join(path)?)
            .header("X-Wonton-Endpoint", self.key.public().to_string())
            .header("X-Wonton-Timestamp", timestamp)
            .header("X-Wonton-Nonce", nonce)
            .header("X-Wonton-Signature", signature)
            .header("Content-Type", "application/json")
            .body(body)
            .send()
            .await
            .context("家名服务连接失败")?;
        let status = response.status();
        ensure!(
            response.content_length().unwrap_or(0) <= 64 << 10,
            "家名服务回应过大"
        );
        let mut response = response;
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await? {
            ensure!(bytes.len() + chunk.len() <= 64 << 10, "家名服务回应过大");
            bytes.extend_from_slice(&chunk);
        }
        if !status.is_success() {
            let kind = serde_json::from_slice::<serde_json::Value>(&bytes)
                .ok()
                .and_then(|value| value.get("error").cloned())
                .and_then(|error| error.get("kind").cloned())
                .and_then(|kind| kind.as_str().map(str::to_owned));
            bail!(
                "家名服务拒绝请求：HTTP {status}，类别 {}",
                kind.as_deref().unwrap_or("upstream")
            );
        }
        ensure!(
            status != reqwest::StatusCode::ACCEPTED,
            "家名服务操作待生效，不能继续签证书"
        );
        serde_json::from_slice(&bytes).context("家名服务回应无法识别")
    }
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut text = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        write!(&mut text, "{byte:02x}").unwrap();
    }
    text
}
