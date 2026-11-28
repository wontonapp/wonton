//! 经本地代理调核心的媒体接口（ADR-0113 第 2 节）。核心凭隧道身份认出这台设备，每个请求还要带
//! `X-Wonton-Device`（值是这台设备的隧道身份），不带的核心一律拒。
//!
//! 失败一律折成 [`MediaFailure`]：核心回了信封就照它；隧道层报的错、连不上、超时算 `transport_unavailable`；
//! 没有信封的按状态码分（见 [`classify`]）。队列改动的 409 不是失败，体是当前队列（[`QueueReply`]）。

use std::time::Duration;

use reqwest::header::{ACCEPT_LANGUAGE, CONTENT_TYPE};
use reqwest::{Client, Method, RequestBuilder, Response};
use serde::Serialize;
use serde::de::DeserializeOwned;
use url::Url;
use wonton_protocol::media::api::{QueueChange, queue_path};
use wonton_protocol::media::{MediaFailure, MediaFailureCode, Queue, WONTON_DEVICE_HEADER};
use wonton_tunnel::ERROR_HEADER;

use crate::chat::proxy_url;
use crate::home::HomeLink;
use crate::l10n::{CATALOG, language};

/// 控制类请求（建任务、起播、控制、列表）的期限：隧道卡住时按连不上处理，不一直转圈。
const CONTROL_TIMEOUT: Duration = Duration::from_secs(30);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(20);

pub(crate) fn failure(code: MediaFailureCode, message: impl Into<String>) -> MediaFailure {
    MediaFailure {
        code,
        message: message.into(),
        retry_after_secs: None,
    }
}

fn unreachable(error: &reqwest::Error) -> MediaFailure {
    failure(MediaFailureCode::TransportUnavailable, error.to_string())
}

/// 队列改动的回应：改成了，或别处先改了（核心回 409，体是当前队列）。
pub(crate) enum QueueReply {
    Applied(Queue),
    Stale(Queue),
}

/// 这次连接要用的 HTTP 客户端。本地代理换了服务器才变，每次操作开始时取一次。
#[derive(Clone)]
pub(crate) struct MediaApi {
    base: Url,
    client: Client,
    device: String,
}

impl MediaApi {
    pub(crate) fn connect(home: &HomeLink) -> Result<Self, MediaFailure> {
        let not_connected = || {
            failure(
                MediaFailureCode::TransportUnavailable,
                "not connected to the home server yet",
            )
        };
        let base = home.core_url().ok_or_else(not_connected)?;
        let view = home.view();
        let proxy = view.proxy.ok_or_else(not_connected)?;
        let client = Client::builder()
            .pool_max_idle_per_host(0)
            .connect_timeout(CONNECT_TIMEOUT)
            .proxy(
                reqwest::Proxy::all(proxy_url(&proxy))
                    .map_err(|e| failure(MediaFailureCode::Protocol, e.to_string()))?,
            )
            .build()
            .map_err(|e| failure(MediaFailureCode::Protocol, e.to_string()))?;
        Ok(Self::new(base, client, view.device_id))
    }

    pub(crate) fn new(base: Url, client: Client, device: String) -> Self {
        Self {
            base,
            client,
            device,
        }
    }

    /// 这台设备的标识：`X-Wonton-Device` 和取命令的 `device` 都用它。
    pub(crate) fn device(&self) -> &str {
        &self.device
    }

    /// 带媒体接口要的请求头，没有总期限：传字节的请求用。
    pub(crate) fn request(&self, method: Method, path: &str) -> RequestBuilder {
        let url = self
            .base
            .join(path)
            .expect("media api paths are absolute and always join");
        self.client
            .request(method, url)
            .header(WONTON_DEVICE_HEADER, &self.device)
            .header(ACCEPT_LANGUAGE, CATALOG.tag(language()))
    }

    pub(crate) async fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T, MediaFailure> {
        let request = self.request(Method::GET, path).timeout(CONTROL_TIMEOUT);
        read(send(request).await?).await
    }

    pub(crate) async fn post<T: DeserializeOwned>(
        &self,
        path: &str,
        body: &impl Serialize,
    ) -> Result<T, MediaFailure> {
        read(send(self.json(Method::POST, path, body)).await?).await
    }

    /// 只要成功、不要回体的请求（位置、停止回 204）。
    pub(crate) async fn post_empty(
        &self,
        path: &str,
        body: &impl Serialize,
    ) -> Result<(), MediaFailure> {
        send(self.json(Method::POST, path, body)).await.map(drop)
    }

    pub(crate) async fn change_queue(
        &self,
        change: &QueueChange,
    ) -> Result<QueueReply, MediaFailure> {
        let response = self
            .json(Method::POST, &queue_path(), change)
            .send()
            .await
            .map_err(|e| unreachable(&e))?;
        let stale = response.status() == reqwest::StatusCode::CONFLICT
            && !response.headers().contains_key(ERROR_HEADER);
        let response = if stale {
            response
        } else {
            check(response).await?
        };
        let queue = read(response).await?;
        Ok(if stale {
            QueueReply::Stale(queue)
        } else {
            QueueReply::Applied(queue)
        })
    }

    fn json(&self, method: Method, path: &str, body: &impl Serialize) -> RequestBuilder {
        self.request(method, path)
            .timeout(CONTROL_TIMEOUT)
            .header(CONTENT_TYPE, "application/json")
            .body(serde_json::to_vec(body).expect("media request bodies always serialize"))
    }
}

/// 发出去，成功的原样返回，失败的折成信封。
pub(crate) async fn send(request: RequestBuilder) -> Result<Response, MediaFailure> {
    let response = request.send().await.map_err(|e| unreachable(&e))?;
    check(response).await
}

async fn check(response: Response) -> Result<Response, MediaFailure> {
    if response.status().is_success() {
        return Ok(response);
    }
    let status = response.status();
    let tunnel = response
        .headers()
        .get(ERROR_HEADER)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let body = response.bytes().await.unwrap_or_default();
    Err(classify(status.as_u16(), tunnel.as_deref(), &body))
}

/// 没成功的回应算什么：隧道层写了错就是隧道不通；核心回了信封照信封；没有信封的按 ADR-0113 第 9 节的表和
/// 核心回信封时用的状态码反着分，表外的 5xx 算那头出错，其余算协议对不上。原状态码进详情。
pub(crate) fn classify(status: u16, tunnel: Option<&str>, body: &[u8]) -> MediaFailure {
    if let Some(code) = tunnel {
        let category = if code == "unauthorized" {
            MediaFailureCode::PermissionDenied
        } else {
            MediaFailureCode::TransportUnavailable
        };
        return failure(category, format!("tunnel: {code} (HTTP {status})"));
    }
    serde_json::from_slice::<MediaFailure>(body).unwrap_or_else(|_| {
        let code = match status {
            401 => MediaFailureCode::AuthenticationRequired,
            403 => MediaFailureCode::PermissionDenied,
            404 | 410 => MediaFailureCode::MediaGone,
            409 => MediaFailureCode::StaleRevision,
            415 => MediaFailureCode::UnsupportedByDevice,
            422 => MediaFailureCode::SourceRequired,
            429 => MediaFailureCode::RateLimited,
            503 => MediaFailureCode::PluginUnavailable,
            507 => MediaFailureCode::ServerStorageFull,
            500..=599 => MediaFailureCode::UpstreamError,
            _ => MediaFailureCode::Protocol,
        };
        failure(
            code,
            format!("core answered HTTP {status} without a media failure envelope"),
        )
    })
}

pub(crate) async fn read<T: DeserializeOwned>(response: Response) -> Result<T, MediaFailure> {
    let body = response.bytes().await.map_err(|e| unreachable(&e))?;
    serde_json::from_slice(&body).map_err(|e| {
        failure(
            MediaFailureCode::Protocol,
            format!("core response is not the agreed shape: {e}"),
        )
    })
}
