//! 经本地代理调核心的文件接口（ADR-0020）。核心凭隧道身份认出这台已配对设备；每个请求都带 [`FILES_HEADER`]。

use std::time::Duration;

use reqwest::header::HeaderMap;
use reqwest::{Client, Method, RequestBuilder, Response, StatusCode};
use serde::de::DeserializeOwned;
use url::Url;
use wonton_l10n::l10n;
use wonton_protocol::{FILE_ERROR_HEADER, FILES_HEADER, FileErrorBody, FileFailure, FileId};
use wonton_tunnel::ERROR_HEADER;

use crate::chat::proxy_url;
use crate::home::HomeLink;
use crate::l10n::{CATALOG, language};

/// 取信息、授权、问进度这类小请求的期限：隧道卡住时按“连不上”处理，不一直转圈。
const CONTROL_TIMEOUT: Duration = Duration::from_secs(60);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);

pub(super) const TUS_RESUMABLE: &str = "Tus-Resumable";
pub(super) const TUS_VERSION: &str = "1.0.0";
const UPLOADS: &str = "/_wonton/files/uploads/";

/// 一次文件操作没做成。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Problem {
    /// 隧道断了、超时、核心那边暂时没回应：传输暂停，连上再接着做。
    Interrupted(String),
    /// 家里的系统服务不可用（见 [`pausable`]）：传输暂停，说明给主人看。这时隧道是通的，主人要知道不是网络的问题。
    ServicesDown(String),
    /// 有明确原因的失败，再试也一样。
    Failed(FileFailure, String),
    /// 文件已经传完，授权或发进房间这一步没成：等主人点重发。
    LinkNotSent(FileFailure, String),
}

impl Problem {
    pub(super) fn into_parts(self) -> (FileFailure, String) {
        match self {
            Self::Interrupted(message) => (FileFailure::TransportUnavailable, message),
            Self::ServicesDown(message) => (FileFailure::ServiceUnavailable, message),
            Self::Failed(reason, message) | Self::LinkNotSent(reason, message) => (reason, message),
        }
    }
}

pub(super) fn interrupted(error: reqwest::Error) -> Problem {
    Problem::Interrupted(error.to_string())
}

/// 当前这次连接要用的 HTTP 客户端。隧道换连接时本地代理不变，换了服务器才变，所以每次传输开始时取一次。
#[derive(Clone)]
pub(super) struct Api {
    base: Url,
    client: Client,
}

impl Api {
    pub(super) fn connect(home: &HomeLink) -> Result<Self, Problem> {
        let not_connected =
            || Problem::Interrupted(l10n!(CATALOG, language(), "files-api-not-connected"));
        let base = home.core_url().ok_or_else(not_connected)?;
        let proxy = home.view().proxy.ok_or_else(not_connected)?;
        let client = Client::builder()
            // 不复用连接：本地代理到回环地址，建连几乎没有代价；空闲的连接可能刚好被对面关掉，POST、PATCH 不能自动重试。
            .pool_max_idle_per_host(0)
            .connect_timeout(CONNECT_TIMEOUT)
            .proxy(reqwest::Proxy::all(proxy_url(&proxy)).map_err(|e| {
                Problem::Failed(
                    FileFailure::Protocol,
                    l10n!(
                        CATALOG,
                        language(),
                        "files-api-invalid-proxy",
                        reason = e.to_string()
                    ),
                )
            })?)
            .build()
            .map_err(|e| {
                Problem::Failed(
                    FileFailure::Protocol,
                    l10n!(
                        CATALOG,
                        language(),
                        "files-api-http-client",
                        reason = e.to_string()
                    ),
                )
            })?;
        Ok(Self { base, client })
    }

    fn url(&self, path: &str) -> Url {
        self.base
            .join(path)
            .expect("fixed path and file id always join")
    }

    /// 带文件接口要的请求头，没有总期限：传字节的请求用。
    pub(super) fn stream(&self, method: Method, path: &str) -> RequestBuilder {
        self.client
            .request(method, self.url(path))
            .header(FILES_HEADER, "1")
    }

    fn control(&self, method: Method, path: &str) -> RequestBuilder {
        self.stream(method, path).timeout(CONTROL_TIMEOUT)
    }

    pub(super) fn upload_path(id: &FileId) -> String {
        format!("{UPLOADS}{id}")
    }

    /// 带 tus 协议头的控制请求。
    pub(super) fn tus(&self, method: Method, path: &str) -> RequestBuilder {
        self.control(method, path)
            .header(TUS_RESUMABLE, TUS_VERSION)
    }

    pub(super) fn uploads(&self) -> RequestBuilder {
        self.tus(Method::POST, UPLOADS)
    }

    pub(super) fn content_path(id: &FileId) -> String {
        format!("/_wonton/files/{id}/content")
    }

    pub(super) async fn info<T: DeserializeOwned>(&self, id: &FileId) -> Result<T, Problem> {
        let response = self
            .control(Method::GET, &format!("/_wonton/files/{id}"))
            .send()
            .await
            .map_err(interrupted)?;
        let response = checked(response).await?;
        let body = response.bytes().await.map_err(interrupted)?;
        serde_json::from_slice(&body).map_err(|e| {
            Problem::Failed(
                FileFailure::Protocol,
                l10n!(
                    CATALOG,
                    language(),
                    "files-api-invalid-response",
                    reason = e.to_string()
                ),
            )
        })
    }

    pub(super) async fn thumbnail(&self, id: &FileId) -> Result<Vec<u8>, Problem> {
        let response = self
            .control(Method::GET, &format!("/_wonton/files/{id}/thumbnail"))
            .send()
            .await
            .map_err(interrupted)?;
        let bytes = checked(response)
            .await?
            .bytes()
            .await
            .map_err(interrupted)?;
        Ok(bytes.to_vec())
    }

    pub(super) async fn delete(&self, id: &FileId) -> Result<(), Problem> {
        let path = format!("/_wonton/files/{id}");
        self.finish(self.control(Method::DELETE, &path)).await
    }

    pub(super) async fn send_to_devices(&self, id: &FileId) -> Result<(), Problem> {
        let path = format!("/_wonton/files/{id}/send-to-devices");
        self.finish(self.control(Method::POST, &path)).await
    }

    pub(super) async fn grant(&self, id: &FileId, room: &str) -> Result<(), Problem> {
        let path = format!("/_wonton/files/{id}/grants");
        let body = wonton_protocol::FileGrantRequest {
            room: room.to_owned(),
        };
        let request = self
            .control(Method::POST, &path)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(serde_json::to_vec(&body).expect("request body always serializes"));
        self.finish(request).await
    }

    async fn finish(&self, request: RequestBuilder) -> Result<(), Problem> {
        let response = request.send().await.map_err(interrupted)?;
        checked(response).await.map(drop)
    }
}

/// 成功的响应原样返回，失败的按原因分开：隧道的问题算中断，核心写明了类别的算失败。
pub(super) async fn checked(response: Response) -> Result<Response, Problem> {
    if response.status().is_success() {
        return Ok(response);
    }
    Err(problem(response).await)
}

async fn problem(response: Response) -> Problem {
    let status = response.status();
    let headers = response.headers().clone();
    let tunnel = header(&headers, ERROR_HEADER).map(str::to_owned);
    if let Some(code) = tunnel {
        return if code == "unauthorized" {
            Problem::Failed(
                FileFailure::Unauthenticated,
                l10n!(CATALOG, language(), "files-api-unauthenticated"),
            )
        } else {
            Problem::Interrupted(l10n!(
                CATALOG,
                language(),
                "files-api-tunnel-unavailable",
                code = code
            ))
        };
    }
    let body = response.bytes().await.unwrap_or_default();
    if let Ok(parsed) = serde_json::from_slice::<FileErrorBody>(&body) {
        return Problem::Failed(parsed.error.kind, parsed.error.message);
    }
    let named = header(&headers, FILE_ERROR_HEADER)
        .and_then(|kind| serde_json::from_value(serde_json::json!(kind)).ok());
    if let Some(kind) = named {
        return Problem::Failed(
            kind,
            l10n!(
                CATALOG,
                language(),
                "files-api-status",
                status = status.as_u16()
            ),
        );
    }
    unnamed(status)
}

/// 上传创建之后和下载的请求碰上 `service_unavailable`：整套服务重启时，有几秒隧道通了而 tusd 还没起来。暂停后接着传，不算失败，不然主人要
/// 重新选文件从头传。卡片和播放照常报这个类别，所以不放在 [`checked`] 里。
pub(super) fn pausable(problem: Problem) -> Problem {
    match problem {
        Problem::Failed(FileFailure::ServiceUnavailable, message) => Problem::ServicesDown(message),
        other => other,
    }
}

/// 没写类别的响应：tusd 转发过来的，或者路上的代理给的。
fn unnamed(status: StatusCode) -> Problem {
    let message = l10n!(
        CATALOG,
        language(),
        "files-api-status",
        status = status.as_u16()
    );
    match status {
        StatusCode::NOT_FOUND | StatusCode::GONE => {
            Problem::Failed(FileFailure::FileUnknown, message)
        }
        // tus：偏移对不上（另一次请求已经写过）、这个上传正被占着。问一下进度再接着传就好。
        StatusCode::CONFLICT
        | StatusCode::LOCKED
        | StatusCode::BAD_GATEWAY
        | StatusCode::SERVICE_UNAVAILABLE
        | StatusCode::GATEWAY_TIMEOUT => Problem::Interrupted(message),
        _ => Problem::Failed(FileFailure::Protocol, message),
    }
}

pub(super) fn header<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers.get(name)?.to_str().ok()
}
