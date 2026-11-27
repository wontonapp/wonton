//! `media_source` 能力的核心侧客户端（ADR-0113“媒体源契约”）：每个请求都带凭据和 `X-Wonton-User`，
//! 非 2xx 一律折成 [`MediaFailure`]。提供方回了信封就照它的码，没回就按状态码归类；连不上算
//! `plugin_unavailable`，2xx 却读不懂算 `protocol`。
//!
//! 地址和凭据由 [`crate::media::provider`] 选好传进来，客户端不认插件。提供方在宿主网络时走 unix 套接字
//! （ADR-0053 的“访问”）。字节（上传分片、下载、取流）不经这里，由网关流式转发。

use std::time::Duration;

use axum::body::{Body, Bytes};
use http_body_util::{BodyExt, Limited};
use hyper::header::{self, HeaderMap, HeaderValue};
use hyper::{Method, Request, Response, StatusCode};
use serde::Serialize;
use serde::de::DeserializeOwned;
use wonton_protocol::media::provider::{
    ItemsPage, ItemsQuery, PATH_REVOKE, PlaybackProgress, PlaybackStopped, ProviderCapabilities,
    QueueSnapshot, RevokeRequest, SegmentsPage, item_segments_path, playback_progress_path,
    playback_queue_path, playback_stopped_path,
};
use wonton_protocol::media::uploads::{
    ExistingItem, PATH_UPLOADS_CHECK, UPLOAD_CHECK_MAX, UPLOAD_OFFSET_HEADER, UploadCheck,
    UploadCheckResult, UploadFinished, UploadOffset, UploadStart, UploadStarted,
    upload_finish_path, upload_path,
};
use wonton_protocol::media::{
    MediaFailure, MediaFailureCode, PATH_CAPABILITIES, PATH_PLAYBACK, PATH_UPLOADS, PlaybackPlan,
    PlaybackRequest, Segment, WONTON_USER_HEADER,
};

use super::upstreams::Endpoint;
use crate::http::HttpClient;

/// 一次调用等多久。上游（Jellyfin 取播放计划要探片源）偶尔慢，比个人数据宽一些。
const CALL_TIMEOUT: Duration = Duration::from_secs(20);

/// 字节转发等响应头多久，和文件代理同一个数（`gateway::proxy` 的 `RESPONSE_HEADER_DEADLINE`）：冷启动的
/// 转码、慢盘收尾都可能超过 JSON 调用的 20 秒。从请求体写完才起算，大分片本身不受它限。
const BYTES_HEADER_DEADLINE: Duration = Duration::from_secs(120);

/// 响应体上限，和 [`HttpClient`] 解析上游 JSON 的上限一致。条目一页、播放计划都远小于它。
const MAX_BODY_BYTES: usize = 1 << 20;

pub(crate) type Reply<T> = Result<T, MediaFailure>;

/// 续写的结果：偏移对上了（200）还是对不上（409），以及提供方此刻的偏移。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PatchOutcome {
    pub conflict: bool,
    pub offset: u64,
}

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

    /// 这些校验和里哪些已在这个人的库里（ADR-0113 第 9 节的去重）。超过单次上限的分批问，回包合在一起。
    #[allow(dead_code)] // 建备份任务前问去重（M18 的执行层）接上之前没有调用点。
    pub(crate) async fn check_uploads(
        &self,
        user: &str,
        checksums: &[String],
    ) -> Reply<Vec<ExistingItem>> {
        let mut existing = Vec::new();
        for batch in checksums.chunks(UPLOAD_CHECK_MAX) {
            let check = UploadCheck {
                checksums: batch.to_vec(),
            };
            let result: UploadCheckResult = self
                .json(Method::POST, PATH_UPLOADS_CHECK, user, Some(&check))
                .await?;
            existing.extend(result.existing);
        }
        Ok(existing)
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

    // 取消任务时放弃上传、队列推给提供方、片段路由这三处接上之前没有调用点。
    #[allow(dead_code)]
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

    // 取消任务时放弃上传、队列推给提供方、片段路由这三处接上之前没有调用点。
    #[allow(dead_code)]
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

    // 取消任务时放弃上传、队列推给提供方、片段路由这三处接上之前没有调用点。
    #[allow(dead_code)]
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

    // 成员退出家庭时收回凭据；接上成员移除之前只有测试调用。
    #[cfg_attr(not(test), allow(dead_code))]
    /// 让提供方注销这把凭据对应的上游凭据、停掉上游会话。
    pub(crate) async fn revoke(&self, user: &str, name: &str) -> Reply<()> {
        let request = RevokeRequest {
            name: name.to_owned(),
        };
        self.empty(Method::POST, PATH_REVOKE, user, &request).await
    }

    /// 续写一项：请求体流式转过去，带 `Content-Range` 和从哪儿续的 `Upload-Offset`（tus 的写法，提供方按它
    /// 判断对不对得上）。提供方回 200 或 409，体里都是当前偏移。
    pub(crate) async fn patch_upload(
        &self,
        user: &str,
        upload_id: &str,
        at: u64,
        mut headers: HeaderMap,
        body: Body,
    ) -> Reply<PatchOutcome> {
        headers.insert(UPLOAD_OFFSET_HEADER, HeaderValue::from(at));
        let response = self
            .forward(user, Method::PATCH, &upload_path(upload_id), headers, body)
            .await;
        let response = response?;
        let status = response.status();
        let bytes = Limited::new(response.into_body(), MAX_BODY_BYTES)
            .collect()
            .await
            .map_err(|e| {
                failure(
                    MediaFailureCode::PluginUnavailable,
                    format!("媒体源的响应没读完：{e}"),
                )
            })?
            .to_bytes();
        let offset: UploadOffset = serde_json::from_slice(&bytes).map_err(|e| {
            failure(
                MediaFailureCode::Protocol,
                format!("续写的响应不是当前偏移：{e}：{}", snippet(&bytes)),
            )
        })?;
        Ok(PatchOutcome {
            conflict: status == StatusCode::CONFLICT,
            offset: offset.upload_offset,
        })
    }

    /// 字节原样转：`path` 带查询串，`headers` 是要带过去的那几个（`Range`、`If-Range`、`Content-Range`……），
    /// 凭据和 `X-Wonton-User` 由这里加。2xx 与 409 原样回（体是流），其余读成失败信封。
    pub(crate) async fn forward(
        &self,
        user: &str,
        method: Method,
        path: &str,
        mut headers: HeaderMap,
        body: Body,
    ) -> Reply<Response<Body>> {
        let url = self.endpoint.url(path);
        headers.insert(
            header::AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {}", self.key)).map_err(|e| {
                failure(MediaFailureCode::Protocol, format!("凭据写不进请求头：{e}"))
            })?,
        );
        headers.insert(
            WONTON_USER_HEADER,
            HeaderValue::from_str(user).map_err(|e| {
                failure(MediaFailureCode::Protocol, format!("用户写不进请求头：{e}"))
            })?,
        );
        let response = match &self.unix {
            None => {
                let mut request = Request::builder()
                    .method(method)
                    .uri(&url)
                    .body(body)
                    .map_err(|e| {
                        failure(MediaFailureCode::Protocol, format!("请求写不出来：{e}"))
                    })?;
                *request.headers_mut() = headers;
                let response = self
                    .http
                    .send_forwarded(request, Some(BYTES_HEADER_DEADLINE))
                    .await
                    .map_err(|e| {
                        failure(
                            MediaFailureCode::PluginUnavailable,
                            format!("连不上媒体源：{e}"),
                        )
                    })?;
                response.map(Body::new)
            }
            Some(client) => {
                // 有体的（续写）在 reqwest 这边分不开“体还在写”和“等响应头”，只给取字节的设期限。
                let bodyless = matches!(method, Method::GET | Method::HEAD);
                let sent = client
                    .request(method, &url)
                    .headers(headers)
                    .body(reqwest::Body::wrap_stream(body.into_data_stream()))
                    .send();
                let sent = if bodyless {
                    tokio::time::timeout(BYTES_HEADER_DEADLINE, sent)
                        .await
                        .map_err(|_| {
                            failure(
                                MediaFailureCode::PluginUnavailable,
                                format!("媒体源 {} 秒没回响应头", BYTES_HEADER_DEADLINE.as_secs()),
                            )
                        })?
                } else {
                    sent.await
                };
                let response = sent.map_err(|e| {
                    failure(
                        MediaFailureCode::PluginUnavailable,
                        format!("unix 套接字请求失败：{}", crate::http::error_chain(&e)),
                    )
                })?;
                let mut built = Response::builder().status(response.status());
                if let Some(target) = built.headers_mut() {
                    *target = response.headers().clone();
                }
                built
                    .body(Body::from_stream(response.bytes_stream()))
                    .map_err(|e| {
                        failure(MediaFailureCode::Protocol, format!("响应拼不起来：{e}"))
                    })?
            }
        };
        let (mut parts, body) = response.into_parts();
        for name in [
            header::CONNECTION,
            header::TRANSFER_ENCODING,
            header::TRAILER,
            header::UPGRADE,
        ] {
            parts.headers.remove(name);
        }
        parts.headers.remove("keep-alive");
        let response = Response::from_parts(parts, body);
        let status = response.status();
        if status.is_success() || status == StatusCode::CONFLICT {
            return Ok(response);
        }
        let headers = response.headers().clone();
        let body = Limited::new(response.into_body(), MAX_BODY_BYTES)
            .collect()
            .await
            .map(|collected| collected.to_bytes())
            .unwrap_or_default();
        Err(failure_from(status, &headers, &body))
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
            .body(Body::from(body.unwrap_or_default()))
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
    failure_from(raw.status, &raw.headers, &raw.body)
}

pub(crate) fn failure_from(status: StatusCode, headers: &HeaderMap, body: &[u8]) -> MediaFailure {
    let mut failure = serde_json::from_slice::<MediaFailure>(body).unwrap_or_else(|_| {
        failure(
            code_for_status(status),
            format!("HTTP {status}：{}", snippet(body)),
        )
    });
    if failure.code == MediaFailureCode::RateLimited && failure.retry_after_secs.is_none() {
        failure.retry_after_secs = headers.get(header::RETRY_AFTER).and_then(retry_after_secs);
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
