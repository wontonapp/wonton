//! `personal_data` 能力的核心侧客户端（ADR-0098 第 5 节）：每个请求都带数据主体，非 2xx 一律折成
//! [`PersonalDataProblem`]，连不上、回的不是 JSON 也如实写成 [`PersonalDataFailure::ProviderUnavailable`]。
//!
//! 地址由调用方按能力解析好传进来，客户端不认插件；提供方在宿主网络时走 unix 套接字（ADR-0053 的“访问”）。
//! 授权（哪个 Bot 能读）不在这里判定。响应里不认识的枚举取值由协议层的 `serde(other)` 落到 `Other`，
//! 所以这里只反序列化，不逐个检查取值。

use std::time::Duration;

use hyper::{Method, StatusCode, header};
use serde::de::DeserializeOwned;
use serde_json::Value;
use wonton_protocol::{
    CLEANUP_PATH, CleanupRequest, CleanupResult, DataQueryResult, EXPORT_PATH, ExportBundle,
    HEALTH_PATH, HealthSamples, LOCATION_PATH, LocationPoint, PersonalDataFailure,
    PersonalDataProblem, RetentionSetting, WONTON_USER_HEADER, WriteAccepted, data_path,
    query_path,
};

use super::upstreams::Endpoint;
use crate::http::{HttpClient, HttpError, RequestBody};

/// 一次调用等多久。导出和清理动的数据多，给得宽一些。
const CALL_TIMEOUT: Duration = Duration::from_secs(15);
const BULK_TIMEOUT: Duration = Duration::from_secs(60);

/// unix 套接字那条路的响应上限，和 [`HttpClient`] 解析上游 JSON 的上限一致。
const MAX_BODY_BYTES: usize = 1 << 20;

type Reply<T> = Result<T, PersonalDataProblem>;

/// 一个存储提供方的客户端。凭据按使用方签发（ADR-0053），由调用方从连接记录里取。
pub(crate) struct PersonalDataClient {
    endpoint: Endpoint,
    key: String,
    http: HttpClient,
    unix: Option<reqwest::Client>,
}

impl PersonalDataClient {
    pub(crate) fn new(endpoint: Endpoint, key: String, http: HttpClient) -> Reply<Self> {
        let unix = match endpoint.socket() {
            None => None,
            Some(socket) => Some(
                reqwest::Client::builder()
                    .unix_socket(socket.to_path_buf())
                    .no_proxy()
                    .build()
                    .map_err(|e| unavailable(format!("unix 套接字的 HTTP 客户端建不起来：{e}")))?,
            ),
        };
        Ok(Self {
            endpoint,
            key,
            http,
            unix,
        })
    }

    pub(crate) async fn write_location(
        &self,
        user: &str,
        point: &LocationPoint,
    ) -> Reply<WriteAccepted> {
        self.call(Method::POST, LOCATION_PATH, user, Some(point), CALL_TIMEOUT)
            .await
    }

    pub(crate) async fn write_health(
        &self,
        user: &str,
        samples: &HealthSamples,
    ) -> Reply<WriteAccepted> {
        self.call(Method::POST, HEALTH_PATH, user, Some(samples), CALL_TIMEOUT)
            .await
    }

    /// 查一个类型在 `[from_ms, to_ms]` 里的数据。
    pub(crate) async fn query(
        &self,
        user: &str,
        type_name: &str,
        from_ms: u64,
        to_ms: u64,
    ) -> Reply<DataQueryResult> {
        self.call(
            Method::GET,
            &query_path(type_name, from_ms, to_ms),
            user,
            None::<&()>,
            CALL_TIMEOUT,
        )
        .await
    }

    /// 删一个类型在 `[from_ms, to_ms]` 里的数据（撤回时用）。
    pub(crate) async fn delete(
        &self,
        user: &str,
        type_name: &str,
        from_ms: u64,
        to_ms: u64,
    ) -> Reply<CleanupResult> {
        self.call(
            Method::DELETE,
            &data_path(type_name, from_ms, to_ms),
            user,
            None::<&()>,
            BULK_TIMEOUT,
        )
        .await
    }

    pub(crate) async fn export(&self, user: &str) -> Reply<ExportBundle> {
        self.call(Method::GET, EXPORT_PATH, user, None::<&()>, BULK_TIMEOUT)
            .await
    }

    /// 按当前保存期限删到期数据；`Never` 的类型收下后写入即丢。
    pub(crate) async fn cleanup(
        &self,
        user: &str,
        policies: Vec<RetentionSetting>,
    ) -> Reply<CleanupResult> {
        let request = CleanupRequest { policies };
        self.call(
            Method::POST,
            CLEANUP_PATH,
            user,
            Some(&request),
            BULK_TIMEOUT,
        )
        .await
    }

    async fn call<T: DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
        user: &str,
        body: Option<&impl serde::Serialize>,
        timeout: Duration,
    ) -> Reply<T> {
        let body = match body {
            None => None,
            Some(body) => Some(
                serde_json::to_vec(body)
                    .map_err(|e| unavailable(format!("请求体写不成 JSON：{e}")))?,
            ),
        };
        let url = self.endpoint.url(path);
        let (status, value) = match &self.unix {
            None => self.tcp(method, &url, user, body, timeout).await?,
            Some(client) => self.unix(client, method, &url, user, body, timeout).await?,
        };
        if status.is_success() {
            return T::deserialize(&value).map_err(|e| {
                unavailable(format!(
                    "HTTP {status} 的响应不是约定的形状：{e}：{}",
                    snippet(&value)
                ))
            });
        }
        Err(problem(status, value))
    }

    async fn tcp(
        &self,
        method: Method,
        url: &str,
        user: &str,
        body: Option<Vec<u8>>,
        timeout: Duration,
    ) -> Reply<(StatusCode, Value)> {
        let body = match body {
            None => RequestBody::Empty,
            Some(data) => RequestBody::Bytes {
                content_type: "application/json",
                data,
            },
        };
        let headers = [(WONTON_USER_HEADER, user.to_owned())];
        self.http
            .request_json(method, url, Some(&self.key), &headers, body, timeout)
            .await
            .map_err(transport)
    }

    async fn unix(
        &self,
        client: &reqwest::Client,
        method: Method,
        url: &str,
        user: &str,
        body: Option<Vec<u8>>,
        timeout: Duration,
    ) -> Reply<(StatusCode, Value)> {
        let mut request = client
            .request(method, url)
            .header(header::ACCEPT.as_str(), "application/json")
            .header(
                header::AUTHORIZATION.as_str(),
                format!("Bearer {}", self.key),
            )
            .header(WONTON_USER_HEADER, user)
            .header(header::CONTENT_TYPE.as_str(), "application/json")
            .timeout(timeout);
        if let Some(body) = body {
            request = request.body(body);
        }
        let response = request.send().await.map_err(reqwest_error)?;
        let status = response.status();
        if response
            .content_length()
            .is_some_and(|len| len > MAX_BODY_BYTES as u64)
        {
            return Err(unavailable(format!("上游响应超过 {MAX_BODY_BYTES} 字节")));
        }
        let bytes = response
            .bytes()
            .await
            .map_err(|e| unavailable(format!("响应体读取失败：{e}")))?;
        let value = if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes)
                .map_err(|e| unavailable(format!("响应不是合法的 JSON：{e}")))?
        };
        Ok((status, value))
    }
}

fn transport(error: HttpError) -> PersonalDataProblem {
    unavailable(error.to_string())
}

fn reqwest_error(error: reqwest::Error) -> PersonalDataProblem {
    let mut text = error.to_string();
    let mut cause = std::error::Error::source(&error);
    while let Some(inner) = cause {
        text.push_str(&format!("：{inner}"));
        cause = inner.source();
    }
    unavailable(format!("unix 套接字请求失败：{text}"))
}

fn unavailable(message: impl Into<String>) -> PersonalDataProblem {
    PersonalDataProblem {
        reason: PersonalDataFailure::ProviderUnavailable,
        message: message.into(),
    }
}

/// 提供方回的错误：能读成 [`PersonalDataProblem`] 就原样带走，读不成也带上状态码和一小段原文。
fn problem(status: StatusCode, value: Value) -> PersonalDataProblem {
    serde_json::from_value(value.clone())
        .unwrap_or_else(|_| unavailable(format!("HTTP {status}：{}", snippet(&value))))
}

fn snippet(value: &Value) -> String {
    let text = value.to_string();
    match text.char_indices().nth(200) {
        None => text,
        Some((index, _)) => format!("{}…", &text[..index]),
    }
}
