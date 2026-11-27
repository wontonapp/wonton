//! `media_source` 能力的核心侧客户端（ADR-0113“媒体源契约”）：每个请求都带凭据和 `X-Wonton-User`，
//! 非 2xx 一律折成 [`MediaFailure`]。提供方回了信封就照它的码，没回就按状态码归类；连不上算
//! `plugin_unavailable`，2xx 却读不懂算 `protocol`。
//!
//! 地址和凭据由 [`crate::media::provider`] 选好传进来，客户端不认插件。提供方在宿主网络时走 unix 套接字
//! （ADR-0053 的“访问”）。字节（上传分片、下载、取流）不经这里，由网关流式转发。

use std::time::Duration;

use axum::body::Bytes;
use http_body_util::{BodyExt, Limited};
use hyper::header::{self, HeaderMap, HeaderValue};
use hyper::{Method, Request, StatusCode};
use serde::Serialize;
use serde::de::DeserializeOwned;
use wonton_protocol::media::provider::{
    ItemsPage, ItemsQuery, PATH_REVOKE, PlaybackProgress, PlaybackStopped, ProviderCapabilities,
    QueueSnapshot, RevokeRequest, SegmentsPage, item_segments_path, playback_progress_path,
    playback_queue_path, playback_stopped_path,
};
use wonton_protocol::media::uploads::{
    UPLOAD_OFFSET_HEADER, UploadFinished, UploadStart, UploadStarted, upload_finish_path,
    upload_path,
};
use wonton_protocol::media::{
    MediaFailure, MediaFailureCode, PATH_CAPABILITIES, PATH_PLAYBACK, PATH_UPLOADS, PlaybackPlan,
    PlaybackRequest, Segment, WONTON_USER_HEADER,
};

use super::upstreams::Endpoint;
use crate::http::HttpClient;

/// 一次调用等多久。上游（Jellyfin 取播放计划要探片源）偶尔慢，比个人数据宽一些。
const CALL_TIMEOUT: Duration = Duration::from_secs(20);

/// 响应体上限，和 [`HttpClient`] 解析上游 JSON 的上限一致。条目一页、播放计划都远小于它。
const MAX_BODY_BYTES: usize = 1 << 20;

pub(crate) type Reply<T> = Result<T, MediaFailure>;

/// 一次往返的原样结果。
struct Raw {
    status: StatusCode,
    headers: HeaderMap,
    body: Bytes,
}

/// 一个媒体源提供方的客户端，绑一把凭据（按人签，见 [`crate::media::provider`]）。
pub(crate) struct MediaSourceClient {
    endpoint: Endpoint,
    key: String,
    http: HttpClient,
    unix: Option<reqwest::Client>,
}

impl MediaSourceClient {
    pub(crate) fn new(endpoint: Endpoint, key: String, http: HttpClient) -> Reply<Self> {
        let unix = match endpoint.socket() {
            None => None,
            Some(socket) => Some(
                reqwest::Client::builder()
                    .unix_socket(socket.to_path_buf())
                    .no_proxy()
                    .build()
                    .map_err(|e| {
                        failure(
                            MediaFailureCode::PluginUnavailable,
                            format!("unix 套接字的 HTTP 客户端建不起来：{e}"),
                        )
                    })?,
            ),
        };
        Ok(Self {
            endpoint,
            key,
            http,
            unix,
        })
    }

    pub(crate) fn endpoint(&self) -> &Endpoint {
        &self.endpoint
    }

    pub(crate) fn key(&self) -> &str {
        &self.key
    }

    pub(crate) async fn capabilities(&self, user: &str) -> Reply<ProviderCapabilities> {
        self.json(Method::GET, PATH_CAPABILITIES, user, None::<&()>)
            .await
    }

    pub(crate) async fn items(&self, user: &str, query: &ItemsQuery) -> Reply<ItemsPage> {
        self.json(Method::GET, &query.path(), user, None::<&()>)
            .await
    }

    pub(crate) async fn start_upload(
        &self,
        user: &str,
        start: &UploadStart,
    ) -> Reply<UploadStarted> {
        self.json(Method::POST, PATH_UPLOADS, user, Some(start))
            .await
    }

    /// 查断点；提供方不认这个上传号（404）时是 `None`，调用方重开一个。
    pub(crate) async fn upload_offset(&self, user: &str, upload_id: &str) -> Reply<Option<u64>> {
        let raw = self
            .exchange(Method::HEAD, &upload_path(upload_id), user, None)
            .await?;
        if raw.status == StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if !raw.status.is_success() {
            return Err(problem(&raw));
        }
        raw.headers
            .get(UPLOAD_OFFSET_HEADER)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.trim().parse().ok())
            .map(Some)
            .ok_or_else(|| {
                failure(
                    MediaFailureCode::Protocol,
                    format!("HEAD 的响应没有合法的 {UPLOAD_OFFSET_HEADER}"),
                )
            })
    }

    pub(crate) async fn finish_upload(&self, user: &str, upload_id: &str) -> Reply<UploadFinished> {
        self.json(
            Method::POST,
            &upload_finish_path(upload_id),
            user,
            None::<&()>,
        )
        .await
    }

    /// 放弃一个上传；提供方说已经没有了（404）也算放弃成了。
    pub(crate) async fn abort_upload(&self, user: &str, upload_id: &str) -> Reply<()> {
        let raw = self
            .exchange(Method::DELETE, &upload_path(upload_id), user, None)
            .await?;
        if raw.status.is_success() || raw.status == StatusCode::NOT_FOUND {
            return Ok(());
        }
        Err(problem(&raw))
    }

    pub(crate) async fn playback(
        &self,
        user: &str,
        request: &PlaybackRequest,
    ) -> Reply<PlaybackPlan> {
        self.json(Method::POST, PATH_PLAYBACK, user, Some(request))
            .await
    }

    pub(crate) async fn progress(
        &self,
        user: &str,
        session: &str,
        progress: &PlaybackProgress,
    ) -> Reply<()> {
        self.empty(
            Method::POST,
            &playback_progress_path(session),
            user,
            progress,
        )
        .await
    }

    pub(crate) async fn save_queue(
        &self,
        user: &str,
        session: &str,
        queue: &QueueSnapshot,
    ) -> Reply<()> {
        self.empty(Method::PUT, &playback_queue_path(session), user, queue)
            .await
    }

    pub(crate) async fn stopped(
        &self,
        user: &str,
        session: &str,
        stopped: &PlaybackStopped,
    ) -> Reply<()> {
        self.empty(Method::POST, &playback_stopped_path(session), user, stopped)
            .await
    }

    /// 章节、片头片尾；提供方没有（404）就是空的。
    pub(crate) async fn segments(&self, user: &str, item: &str) -> Reply<Vec<Segment>> {
        let raw = self
            .exchange(Method::GET, &item_segments_path(item), user, None)
            .await?;
        if raw.status == StatusCode::NOT_FOUND {
            return Ok(Vec::new());
        }
        decode::<SegmentsPage>(&raw).map(|page| page.segments)
    }

    /// 让提供方注销这把凭据对应的上游凭据、停掉上游会话。
    pub(crate) async fn revoke(&self, user: &str, name: &str) -> Reply<()> {
        let request = RevokeRequest {
            name: name.to_owned(),
        };
        self.empty(Method::POST, PATH_REVOKE, user, &request).await
    }

    async fn json<T: DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
        user: &str,
        body: Option<&impl Serialize>,
    ) -> Reply<T> {
        let body = body.map(to_json).transpose()?;
        let raw = self.exchange(method, path, user, body).await?;
        decode(&raw)
    }

    /// 成功不要响应体的调用（204 或者随便什么 2xx）。
    async fn empty(
        &self,
        method: Method,
        path: &str,
        user: &str,
        body: &impl Serialize,
    ) -> Reply<()> {
        let raw = self
            .exchange(method, path, user, Some(to_json(body)?))
            .await?;
        if raw.status.is_success() {
            return Ok(());
        }
        Err(problem(&raw))
    }

    async fn exchange(
        &self,
        method: Method,
        path: &str,
        user: &str,
        body: Option<Vec<u8>>,
    ) -> Reply<Raw> {
        let url = self.endpoint.url(path);
        match &self.unix {
            None => self.tcp(method, &url, user, body).await,
            Some(client) => self.unix(client, method, &url, user, body).await,
        }
    }

    async fn tcp(
        &self,
        method: Method,
        url: &str,
        user: &str,
        body: Option<Vec<u8>>,
    ) -> Reply<Raw> {
        let mut builder = Request::builder()
            .method(method)
            .uri(url)
            .header(header::ACCEPT, "application/json")
            .header(header::AUTHORIZATION, format!("Bearer {}", self.key))
            .header(WONTON_USER_HEADER, user);
        if body.is_some() {
            builder = builder.header(header::CONTENT_TYPE, "application/json");
        }
        let request = builder
            .body(axum::body::Body::from(body.unwrap_or_default()))
            .map_err(|e| failure(MediaFailureCode::Protocol, format!("请求写不出来：{e}")))?;
        let exchange = async {
            let response = self.http.send(request).await.map_err(|e| {
                failure(
                    MediaFailureCode::PluginUnavailable,
                    format!("连不上媒体源：{e}"),
                )
            })?;
            let status = response.status();
            let headers = response.headers().clone();
            let body = Limited::new(response.into_body(), MAX_BODY_BYTES)
                .collect()
                .await
                .map_err(|e| {
                    failure(
                        MediaFailureCode::PluginUnavailable,
                        format!("媒体源的响应没读完：{e}"),
                    )
                })?
                .to_bytes();
            Ok(Raw {
                status,
                headers,
                body,
            })
        };
        tokio::time::timeout(CALL_TIMEOUT, exchange)
            .await
            .unwrap_or_else(|_| {
                Err(failure(
                    MediaFailureCode::PluginUnavailable,
                    format!("媒体源 {} 秒没回话", CALL_TIMEOUT.as_secs()),
                ))
            })
    }

    async fn unix(
        &self,
        client: &reqwest::Client,
        method: Method,
        url: &str,
        user: &str,
        body: Option<Vec<u8>>,
    ) -> Reply<Raw> {
        let mut request = client
            .request(method, url)
            .header(header::ACCEPT.as_str(), "application/json")
            .header(
                header::AUTHORIZATION.as_str(),
                format!("Bearer {}", self.key),
            )
            .header(WONTON_USER_HEADER, user)
            .timeout(CALL_TIMEOUT);
        if let Some(body) = body {
            request = request
                .header(header::CONTENT_TYPE.as_str(), "application/json")
                .body(body);
        }
        let response = request.send().await.map_err(|e| {
            failure(
                MediaFailureCode::PluginUnavailable,
                format!("unix 套接字请求失败：{}", crate::http::error_chain(&e)),
            )
        })?;
        let status = response.status();
        let headers = response.headers().clone();
        if response
            .content_length()
            .is_some_and(|len| len > MAX_BODY_BYTES as u64)
        {
            return Err(failure(
                MediaFailureCode::Protocol,
                format!("媒体源的响应超过 {MAX_BODY_BYTES} 字节"),
            ));
        }
        let body = response.bytes().await.map_err(|e| {
            failure(
                MediaFailureCode::PluginUnavailable,
                format!("媒体源的响应没读完：{e}"),
            )
        })?;
        Ok(Raw {
            status,
            headers,
            body,
        })
    }
}

fn to_json(body: &impl Serialize) -> Reply<Vec<u8>> {
    serde_json::to_vec(body).map_err(|e| {
        failure(
            MediaFailureCode::Protocol,
            format!("请求体写不成 JSON：{e}"),
        )
    })
}

fn decode<T: DeserializeOwned>(raw: &Raw) -> Reply<T> {
    if !raw.status.is_success() {
        return Err(problem(raw));
    }
    serde_json::from_slice(&raw.body).map_err(|e| {
        failure(
            MediaFailureCode::Protocol,
            format!(
                "HTTP {} 的响应不是约定的形状：{e}：{}",
                raw.status,
                snippet(&raw.body)
            ),
        )
    })
}

pub(crate) fn failure(code: MediaFailureCode, message: impl Into<String>) -> MediaFailure {
    MediaFailure {
        code,
        message: message.into(),
        retry_after_secs: None,
    }
}

/// 提供方回的失败：读得成信封就照它，读不成按状态码归类、带上一小段原文。限速的等多久，信封没写就看
/// `Retry-After`。
fn problem(raw: &Raw) -> MediaFailure {
    let mut failure = serde_json::from_slice::<MediaFailure>(&raw.body).unwrap_or_else(|_| {
        failure(
            code_for_status(raw.status),
            format!("HTTP {}：{}", raw.status, snippet(&raw.body)),
        )
    });
    if failure.code == MediaFailureCode::RateLimited && failure.retry_after_secs.is_none() {
        failure.retry_after_secs = raw
            .headers
            .get(header::RETRY_AFTER)
            .and_then(retry_after_secs);
    }
    failure
}

/// 只认秒数写法；HTTP 日期写法交给退避表的默认值。
fn retry_after_secs(value: &HeaderValue) -> Option<u32> {
    value.to_str().ok()?.trim().parse().ok()
}

/// 没有信封时按状态码归类。401 是上游凭据失效（不退避，要重新登录），404 是条目不在了。
fn code_for_status(status: StatusCode) -> MediaFailureCode {
    match status {
        StatusCode::UNAUTHORIZED => MediaFailureCode::AuthenticationRequired,
        StatusCode::FORBIDDEN => MediaFailureCode::PermissionDenied,
        StatusCode::NOT_FOUND | StatusCode::GONE => MediaFailureCode::MediaGone,
        StatusCode::UNSUPPORTED_MEDIA_TYPE | StatusCode::NOT_ACCEPTABLE => {
            MediaFailureCode::UnsupportedByDevice
        }
        StatusCode::TOO_MANY_REQUESTS => MediaFailureCode::RateLimited,
        StatusCode::INSUFFICIENT_STORAGE | StatusCode::PAYLOAD_TOO_LARGE => {
            MediaFailureCode::ServerStorageFull
        }
        status if status.is_server_error() => MediaFailureCode::UpstreamError,
        _ => MediaFailureCode::Protocol,
    }
}

fn snippet(body: &[u8]) -> String {
    let text = String::from_utf8_lossy(body);
    match text.char_indices().nth(200) {
        None => text.into_owned(),
        Some((index, _)) => format!("{}…", &text[..index]),
    }
}
