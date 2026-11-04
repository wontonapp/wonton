//! 核心的插件管理接口（ADR-0017 的“管理接口”）：请求走本地代理，核心凭隧道身份认出这台已配对设备。请求不带
//! `Origin` 和 `Sec-Fetch-Site`，核心据此知道它不是网页发起的。

use std::time::Duration;

use reqwest::header::{ACCEPT_LANGUAGE, CONTENT_TYPE};
use reqwest::{Client, Method};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use url::Url;
use wonton_protocol::{CoreError, PluginFailure, PluginImage, PluginProblem};

use crate::chat::proxy_url;

/// 列表、详情、图片：只读家里服务器上已有的东西。隧道卡住时按“连不上”失败，不一直转圈。
pub(super) const READ_TIMEOUT: Duration = Duration::from_secs(30);
/// 家里的服务器要去来源（GitHub）取东西：商店、商店里的详情、预览。
pub(super) const SOURCE_TIMEOUT: Duration = Duration::from_secs(120);
/// 安装和升级：响应要等拉完镜像、通过健康检查。Hermes 的镜像约 1 GB，慢的网络要很久；超时后核心照样做完，
/// 结果从已安装列表里看。
pub(super) const INSTALL_TIMEOUT: Duration = Duration::from_secs(30 * 60);

const IMAGE_PREFIX: &str = "/_wonton/api/images/";

/// 核心的错误响应：`{"error": {"kind": …, "message": …}}`。
#[derive(Deserialize)]
struct ErrorBody {
    error: ErrorDetail,
}

#[derive(Deserialize)]
struct ErrorDetail {
    kind: String,
    message: String,
}

/// 一次请求要用的连接信息：核心的地址、本地代理，以及主人的语言。
#[derive(Clone)]
pub(super) struct CoreApi {
    pub(super) base: Url,
    pub(super) client: Client,
    pub(super) languages: String,
}

/// 建本地代理的 HTTP 客户端。
pub(super) fn proxied_client(
    proxy: &wonton_protocol::ProxySettings,
) -> Result<Client, PluginProblem> {
    let protocol = |what: &str, e: reqwest::Error| PluginProblem {
        reason: PluginFailure::Protocol,
        message: format!("{what}：{e}"),
    };
    Client::builder()
        // 不复用连接：本地代理到回环地址，建连几乎没有代价；空闲的连接可能刚好被对面关掉，POST 不能自动重试，
        // 安装就会因此按“连接断了”处理。
        .pool_max_idle_per_host(0)
        .proxy(
            reqwest::Proxy::all(proxy_url(proxy)).map_err(|e| protocol("本地代理地址不合法", e))?,
        )
        .build()
        .map_err(|e| protocol("建 HTTP 客户端失败", e))
}

impl CoreApi {
    /// `/_wonton/api/<segments…>?<query…>`：片段和参数都做转义，插件 ID 和来源里的字符不会改变路径。
    fn url(&self, segments: &[&str], query: &[(&str, &str)]) -> Url {
        let mut url = self.base.clone();
        url.path_segments_mut()
            .expect("核心的地址是 http")
            .clear()
            .extend(["_wonton", "api"])
            .extend(segments);
        if !query.is_empty() {
            url.query_pairs_mut().extend_pairs(query);
        }
        url
    }

    async fn send(
        &self,
        method: Method,
        url: Url,
        body: Option<Vec<u8>>,
        timeout: Duration,
    ) -> Result<Vec<u8>, PluginProblem> {
        let mut request = self.client.request(method, url).timeout(timeout);
        if !self.languages.is_empty() {
            request = request.header(ACCEPT_LANGUAGE, &self.languages);
        }
        if let Some(body) = body {
            request = request.header(CONTENT_TYPE, "application/json").body(body);
        }
        let response = request.send().await.map_err(|e| transport(&e, timeout))?;
        let status = response.status().as_u16();
        let bytes = response.bytes().await.map_err(|e| transport(&e, timeout))?;
        if status >= 400 {
            return Err(core_error(status, &bytes));
        }
        Ok(bytes.to_vec())
    }

    pub(super) async fn get<T: DeserializeOwned>(
        &self,
        segments: &[&str],
        query: &[(&str, &str)],
        timeout: Duration,
    ) -> Result<T, PluginProblem> {
        let bytes = self
            .send(Method::GET, self.url(segments, query), None, timeout)
            .await?;
        parse(&bytes)
    }

    /// 发 JSON 请求体，响应是 JSON。
    pub(super) async fn post<T: DeserializeOwned>(
        &self,
        segments: &[&str],
        body: &impl serde::Serialize,
        timeout: Duration,
    ) -> Result<T, PluginProblem> {
        let body = serde_json::to_vec(body).map_err(|e| PluginProblem {
            reason: PluginFailure::Protocol,
            message: format!("请求体写不出来：{e}"),
        })?;
        let bytes = self
            .send(Method::POST, self.url(segments, &[]), Some(body), timeout)
            .await?;
        parse(&bytes)
    }

    /// 发 JSON 请求体，响应是 JSON。
    pub(super) async fn put<T: DeserializeOwned>(
        &self,
        segments: &[&str],
        body: &impl serde::Serialize,
        timeout: Duration,
    ) -> Result<T, PluginProblem> {
        let body = serde_json::to_vec(body).map_err(|e| PluginProblem {
            reason: PluginFailure::Protocol,
            message: format!("请求体写不出来：{e}"),
        })?;
        let bytes = self
            .send(Method::PUT, self.url(segments, &[]), Some(body), timeout)
            .await?;
        parse(&bytes)
    }

    /// 只看成败，不解析响应：安装的结果以已安装列表为准，核心比 App 新时响应的形状可以变。
    pub(super) async fn post_unit(
        &self,
        segments: &[&str],
        body: &impl serde::Serialize,
        timeout: Duration,
    ) -> Result<(), PluginProblem> {
        let body = serde_json::to_vec(body).map_err(|e| PluginProblem {
            reason: PluginFailure::Protocol,
            message: format!("请求体写不出来：{e}"),
        })?;
        self.send(Method::POST, self.url(segments, &[]), Some(body), timeout)
            .await
            .map(drop)
    }

    pub(super) async fn delete(
        &self,
        segments: &[&str],
        query: &[(&str, &str)],
        timeout: Duration,
    ) -> Result<(), PluginProblem> {
        self.send(Method::DELETE, self.url(segments, query), None, timeout)
            .await
            .map(drop)
    }

    /// `PluginImage.path` 是核心给的地址，只取图片接口下面的。
    pub(super) async fn image(&self, image: &PluginImage) -> Result<Vec<u8>, PluginProblem> {
        if !image.path.starts_with(IMAGE_PREFIX) {
            return Err(PluginProblem {
                reason: PluginFailure::Invalid,
                message: format!("不是插件图片的地址：{}", image.path),
            });
        }
        let url = self.base.join(&image.path).map_err(|e| PluginProblem {
            reason: PluginFailure::Invalid,
            message: format!("不是插件图片的地址：{e}"),
        })?;
        // 上面看的是规范化之前的写法，`images/../devices` 也能通过。
        if !url.path().starts_with(IMAGE_PREFIX) {
            return Err(PluginProblem {
                reason: PluginFailure::Invalid,
                message: format!("不是插件图片的地址：{}", image.path),
            });
        }
        self.send(Method::GET, url, None, READ_TIMEOUT).await
    }
}

fn parse<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, PluginProblem> {
    serde_json::from_slice(bytes).map_err(|e| PluginProblem {
        reason: PluginFailure::Protocol,
        message: format!("家里的服务器给的内容格式不对：{e}"),
    })
}

fn transport(error: &reqwest::Error, timeout: Duration) -> PluginProblem {
    let reason = if error.is_timeout() {
        format!("{} 秒内没有回应", timeout.as_secs())
    } else {
        error.to_string()
    };
    PluginProblem {
        reason: PluginFailure::TransportUnavailable,
        message: format!("连不上家里的服务器：{reason}"),
    }
}

/// 按核心的错误类别区分原因（ADR-0017）；没有错误体的响应来自核心之外，比如本地代理连不上隧道时回的 502。
fn core_error(status: u16, body: &[u8]) -> PluginProblem {
    let detail = serde_json::from_slice::<ErrorBody>(body)
        .ok()
        .map(|body| body.error);
    let reason = match detail.as_ref().map(|detail| detail.kind.as_str()) {
        Some("unauthenticated" | "cross_site") => PluginFailure::Unauthorized,
        // 授权列表和插件清单对不上，和设备身份无关：重新看预览、确认，重新配对解决不了。
        Some("changed" | "permission_denied") => PluginFailure::Changed,
        Some("source_unavailable") => PluginFailure::SourceUnavailable,
        Some("not_found" | "not_installed") => PluginFailure::NotFound,
        Some("unsupported" | "gpu_unavailable") => PluginFailure::Unsupported,
        Some("conflict") => PluginFailure::Conflict,
        Some("invalid_input") => PluginFailure::Invalid,
        Some("lan_unavailable") => PluginFailure::LanUnavailable,
        Some("egress_proxy_unavailable") => PluginFailure::EgressProxyUnavailable,
        Some("core_upgrading") => PluginFailure::CoreUpgrading,
        Some(_) if status >= 500 => PluginFailure::Upstream,
        Some(_) => PluginFailure::Protocol,
        None => match status {
            401 | 403 => PluginFailure::Unauthorized,
            502 => PluginFailure::TransportUnavailable,
            500..=599 => PluginFailure::Upstream,
            _ => PluginFailure::Protocol,
        },
    };
    let message = detail.map_or_else(
        || format!("家里的服务器回了 {status}"),
        |detail| detail.message,
    );
    PluginProblem { reason, message }
}

/// 直接返回结果的调用（详情、预览、图片）按 [`CoreError`] 的分类失败，核心写的说明原样带上。
pub(super) fn to_core_error(problem: PluginProblem) -> CoreError {
    let reason = problem.message;
    match problem.reason {
        PluginFailure::TransportUnavailable => CoreError::TransportUnavailable { reason },
        PluginFailure::Unauthorized => CoreError::Unauthorized { reason },
        PluginFailure::NotFound | PluginFailure::Invalid | PluginFailure::Changed => {
            CoreError::InvalidInput { reason }
        }
        PluginFailure::Unsupported
        | PluginFailure::Conflict
        | PluginFailure::LanUnavailable
        | PluginFailure::EgressProxyUnavailable
        | PluginFailure::CoreUpgrading => CoreError::NotReady { reason },
        PluginFailure::SourceUnavailable | PluginFailure::Upstream | PluginFailure::Protocol => {
            CoreError::Upstream { reason }
        }
    }
}
