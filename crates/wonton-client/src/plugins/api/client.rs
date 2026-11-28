//! 走本地代理的插件管理请求：地址、超时、发送与解析，传输失败统一成“连不上”。

use std::time::Duration;

use reqwest::header::{ACCEPT_LANGUAGE, CONTENT_TYPE};
use reqwest::{Client, Method};
use serde::de::DeserializeOwned;
use url::Url;
use wonton_l10n::l10n;
use wonton_protocol::{
    CoreUpgradeFailure, CoreUpgradeProblem, CoreUpgradeStatus, PluginFailure, PluginImage,
    PluginProblem, minimum_server_version, window_path,
};

use super::ServerVersion;
use super::error::core_error;
use super::response::{CoreApiError, http_error, upgrade_error};
use crate::chat::proxy_url;
use crate::l10n::{CATALOG, language};

/// 列表、详情、图片：只读家里服务器上已有的东西。隧道卡住时按“连不上”失败，不一直转圈。
pub(crate) const READ_TIMEOUT: Duration = Duration::from_secs(30);
/// 家里的服务器要去来源（GitHub）取东西：商店、商店里的详情、预览，以及引导要取的官方索引和图标。
pub(crate) const SOURCE_TIMEOUT: Duration = Duration::from_secs(120);
/// 安装和升级：响应要等拉完镜像、通过健康检查。Hermes 的镜像约 1 GB，慢的网络要很久；超时后核心照样做完，
/// 结果从已安装列表里看。核心等健康至多 60 分钟（按插件的 healthcheck.start_period 放宽），再留出拉镜像的时间。
pub(in crate::plugins) const INSTALL_TIMEOUT: Duration = Duration::from_secs(90 * 60);

const IMAGE_PREFIX: &str = "/_wonton/api/images/";

/// 一次请求要用的连接信息：核心的地址、本地代理，以及主人的语言。
#[derive(Clone)]
pub(crate) struct CoreApi {
    pub(in crate::plugins) base: Url,
    pub(in crate::plugins) client: Client,
    pub(in crate::plugins) languages: String,
    /// 这台服务器报过的版本。只有接口回 404 时才去取一次，取不到就不判旧。
    pub(in crate::plugins) version: ServerVersion,
}

/// 建本地代理的 HTTP 客户端。
pub(crate) fn proxied_client(
    proxy: &wonton_protocol::ProxySettings,
) -> Result<Client, PluginProblem> {
    Client::builder()
        // 不复用连接：本地代理到回环地址，建连几乎没有代价；空闲的连接可能刚好被对面关掉，POST 不能自动重试，
        // 安装就会因此按“连接断了”处理。
        .pool_max_idle_per_host(0)
        .proxy(
            reqwest::Proxy::all(proxy_url(proxy)).map_err(|e| PluginProblem {
                reason: PluginFailure::Protocol,
                min_version: None,
                message: l10n!(
                    CATALOG,
                    language(),
                    "plugins-api-invalid-proxy",
                    reason = e.to_string()
                ),
                settings: None,
            })?,
        )
        .build()
        .map_err(|e| PluginProblem {
            reason: PluginFailure::Protocol,
            min_version: None,
            message: l10n!(
                CATALOG,
                language(),
                "plugins-api-http-client",
                reason = e.to_string()
            ),
            settings: None,
        })
}

impl CoreApi {
    /// `/_wonton/api/<segments…>?<query…>`：片段和参数都做转义，插件 ID 和来源里的字符不会改变路径。
    pub(crate) fn url(&self, segments: &[&str], query: &[(&str, &str)]) -> Url {
        let mut url = self.base.clone();
        url.path_segments_mut()
            .expect("home server URL is http")
            .clear()
            .extend(["_wonton", "api"])
            .extend(segments);
        if !query.is_empty() {
            url.query_pairs_mut().extend_pairs(query);
        }
        url
    }

    /// 发一次请求，回状态码、`X-Wonton-Error` 类别（没有为空）和响应体。成员接口自己按类别分错误，
    /// 不套用插件的分类。传输失败仍然是 [`PluginProblem`]，由调用方转成自己的类别。
    pub(crate) async fn exchange_detail(
        &self,
        method: Method,
        url: Url,
        body: Option<Vec<u8>>,
        content_type: &str,
        timeout: Duration,
    ) -> Result<(u16, Option<String>, Vec<u8>), PluginProblem> {
        let mut request = self.client.request(method, url).timeout(timeout);
        if !self.languages.is_empty() {
            request = request.header(ACCEPT_LANGUAGE, &self.languages);
        }
        if let Some(body) = body {
            request = request.header(CONTENT_TYPE, content_type).body(body);
        }
        let response = request.send().await.map_err(|e| transport(&e, timeout))?;
        let status = response.status().as_u16();
        let kind = response
            .headers()
            .get("x-wonton-error")
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let bytes = response.bytes().await.map_err(|e| transport(&e, timeout))?;
        Ok((status, kind, bytes.to_vec()))
    }

    pub(super) async fn exchange(
        &self,
        method: Method,
        url: Url,
        body: Option<Vec<u8>>,
        content_type: &str,
        timeout: Duration,
    ) -> Result<(u16, Vec<u8>), PluginProblem> {
        let (status, _, bytes) = self
            .exchange_detail(method, url, body, content_type, timeout)
            .await?;
        Ok((status, bytes))
    }

    async fn send(
        &self,
        method: Method,
        url: Url,
        body: Option<Vec<u8>>,
        timeout: Duration,
    ) -> Result<Vec<u8>, PluginProblem> {
        self.send_typed(method, url, body, "application/json", timeout)
            .await
    }

    async fn send_typed(
        &self,
        method: Method,
        url: Url,
        body: Option<Vec<u8>>,
        content_type: &str,
        timeout: Duration,
    ) -> Result<Vec<u8>, PluginProblem> {
        let path = url.path().to_owned();
        let (status, bytes) = self
            .exchange(method, url, body, content_type, timeout)
            .await?;
        if status >= 400 {
            return Err(self.problem(status, &bytes, &path).await);
        }
        Ok(bytes)
    }

    /// 失败原因：接口在表里、服务器又比要的版本旧时归“服务器太旧”，否则按原来的类别。
    async fn problem(&self, status: u16, body: &[u8], path: &str) -> PluginProblem {
        let minimum = (status == 404)
            .then(|| minimum_server_version(path))
            .flatten();
        let version = match minimum {
            Some(_) => self.version.get(self).await,
            None => None,
        };
        core_error(status, body, minimum, version.as_deref())
    }

    /// 升级的接口都回 [`CoreUpgradeStatus`]；被拒绝的原因（`changed`、`busy`……）比插件的失败分类细，要原样保留。
    pub(crate) async fn core_upgrade(
        &self,
        method: Method,
        segments: &[&str],
        body: Option<Vec<u8>>,
        timeout: Duration,
    ) -> Result<CoreUpgradeStatus, CoreUpgradeProblem> {
        let (status, bytes) = self
            .exchange(
                method,
                self.url(segments, &[]),
                body,
                "application/json",
                timeout,
            )
            .await
            .map_err(|problem| CoreUpgradeProblem {
                reason: CoreUpgradeFailure::TransportUnavailable,
                message: problem.message,
            })?;
        if status >= 400 {
            return Err(upgrade_error(status, &bytes));
        }
        serde_json::from_slice(&bytes).map_err(|e| CoreUpgradeProblem {
            reason: CoreUpgradeFailure::Protocol,
            message: l10n!(
                CATALOG,
                language(),
                "plugins-api-invalid-response",
                reason = e.to_string()
            ),
        })
    }

    pub(crate) async fn get<T: DeserializeOwned>(
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
    pub(crate) async fn post<T: DeserializeOwned>(
        &self,
        segments: &[&str],
        body: &impl serde::Serialize,
        timeout: Duration,
    ) -> Result<T, PluginProblem> {
        let body = serde_json::to_vec(body).map_err(|e| PluginProblem {
            reason: PluginFailure::Protocol,
            min_version: None,
            message: l10n!(
                CATALOG,
                language(),
                "plugins-api-invalid-request",
                reason = e.to_string()
            ),
            settings: None,
        })?;
        let bytes = self
            .send(Method::POST, self.url(segments, &[]), Some(body), timeout)
            .await?;
        parse(&bytes)
    }

    /// 发主人选的压缩包，响应是 JSON。核心按内容认格式，这里不用说是哪一种。
    pub(crate) async fn post_archive<T: DeserializeOwned>(
        &self,
        segments: &[&str],
        archive: Vec<u8>,
        timeout: Duration,
    ) -> Result<T, PluginProblem> {
        let bytes = self
            .send_typed(
                Method::POST,
                self.url(segments, &[]),
                Some(archive),
                "application/octet-stream",
                timeout,
            )
            .await?;
        parse(&bytes)
    }

    /// 发 JSON 请求体，响应是 JSON。
    pub(crate) async fn put<T: DeserializeOwned>(
        &self,
        segments: &[&str],
        body: &impl serde::Serialize,
        timeout: Duration,
    ) -> Result<T, PluginProblem> {
        let body = serde_json::to_vec(body).map_err(|e| PluginProblem {
            reason: PluginFailure::Protocol,
            min_version: None,
            message: l10n!(
                CATALOG,
                language(),
                "plugins-api-invalid-request",
                reason = e.to_string()
            ),
            settings: None,
        })?;
        let bytes = self
            .send(Method::PUT, self.url(segments, &[]), Some(body), timeout)
            .await?;
        parse(&bytes)
    }

    pub(crate) async fn put_detailed<T: DeserializeOwned>(
        &self,
        segments: &[&str],
        body: &impl serde::Serialize,
        timeout: Duration,
    ) -> Result<T, CoreApiError> {
        let body = serde_json::to_vec(body)
            .map_err(|error| CoreApiError::Protocol(format!("Cannot encode request: {error}")))?;
        let (status, bytes) = self
            .exchange(
                Method::PUT,
                self.url(segments, &[]),
                Some(body),
                "application/json",
                timeout,
            )
            .await
            .map_err(CoreApiError::Transport)?;
        if status >= 400 {
            return Err(http_error(status, &bytes));
        }
        serde_json::from_slice(&bytes)
            .map_err(|error| CoreApiError::Protocol(format!("Invalid core response: {error}")))
    }

    /// `GET`，响应是原始字节；错误带类别和消息。链接预览的图片用（ADR-0065）。
    pub(crate) async fn get_bytes_detailed(
        &self,
        segments: &[&str],
        timeout: Duration,
    ) -> Result<Vec<u8>, CoreApiError> {
        let (status, bytes) = self
            .exchange(
                Method::GET,
                self.url(segments, &[]),
                None,
                "application/json",
                timeout,
            )
            .await
            .map_err(CoreApiError::Transport)?;
        if status < 400 {
            return Ok(bytes);
        }
        Err(http_error(status, &bytes))
    }

    /// `GET` 但错误带类别和消息：取模型服务商列表要分得清网关没就绪和别的失败，响应本身还是 JSON。
    pub(crate) async fn get_detailed<T: DeserializeOwned>(
        &self,
        segments: &[&str],
        query: &[(&str, &str)],
        timeout: Duration,
    ) -> Result<T, CoreApiError> {
        let (status, bytes) = self
            .exchange(
                Method::GET,
                self.url(segments, query),
                None,
                "application/json",
                timeout,
            )
            .await
            .map_err(CoreApiError::Transport)?;
        if status < 400 {
            return serde_json::from_slice(&bytes).map_err(|error| {
                CoreApiError::Protocol(l10n!(
                    CATALOG,
                    language(),
                    "plugins-api-invalid-response",
                    reason = error.to_string()
                ))
            });
        }
        Err(http_error(status, &bytes))
    }

    /// 没有请求体，响应是 JSON。
    pub(crate) async fn empty<T: DeserializeOwned>(
        &self,
        method: Method,
        segments: &[&str],
        timeout: Duration,
    ) -> Result<T, PluginProblem> {
        let bytes = self
            .send(method, self.url(segments, &[]), None, timeout)
            .await?;
        parse(&bytes)
    }

    /// 停止放行时段。核心回 404 说明已经结束了，主人要的就是这个。
    pub(crate) async fn stop_window(
        &self,
        card: &str,
        timeout: Duration,
    ) -> Result<(), PluginProblem> {
        let mut url = self.base.clone();
        url.set_path(&window_path(card));
        let (status, bytes) = self
            .exchange(Method::DELETE, url, None, "application/json", timeout)
            .await?;
        if status >= 400 && status != 404 {
            return Err(core_error(status, &bytes, None, None));
        }
        Ok(())
    }

    /// 只看成败，不解析响应：安装的结果以已安装列表为准，核心比 App 新时响应的形状可以变。
    pub(crate) async fn post_unit(
        &self,
        segments: &[&str],
        body: &impl serde::Serialize,
        timeout: Duration,
    ) -> Result<(), PluginProblem> {
        self.post_raw(segments, body, timeout).await.map(drop)
    }

    pub(crate) async fn post_raw(
        &self,
        segments: &[&str],
        body: &impl serde::Serialize,
        timeout: Duration,
    ) -> Result<Vec<u8>, PluginProblem> {
        let body = serde_json::to_vec(body).map_err(|e| PluginProblem {
            reason: PluginFailure::Protocol,
            min_version: None,
            message: l10n!(
                CATALOG,
                language(),
                "plugins-api-invalid-request",
                reason = e.to_string()
            ),
            settings: None,
        })?;
        self.send(Method::POST, self.url(segments, &[]), Some(body), timeout)
            .await
    }

    /// 发 JSON 请求体，响应是空的。
    pub(crate) async fn put_unit_body(
        &self,
        segments: &[&str],
        body: &impl serde::Serialize,
        timeout: Duration,
    ) -> Result<(), PluginProblem> {
        let body = serde_json::to_vec(body).map_err(|e| PluginProblem {
            reason: PluginFailure::Protocol,
            min_version: None,
            message: l10n!(
                CATALOG,
                language(),
                "plugins-api-invalid-request",
                reason = e.to_string()
            ),
            settings: None,
        })?;
        self.send(Method::PUT, self.url(segments, &[]), Some(body), timeout)
            .await
            .map(drop)
    }

    /// 响应是空的。
    pub(crate) async fn put_unit(
        &self,
        segments: &[&str],
        timeout: Duration,
    ) -> Result<(), PluginProblem> {
        self.send(Method::PUT, self.url(segments, &[]), None, timeout)
            .await
            .map(drop)
    }

    pub(crate) async fn delete(
        &self,
        segments: &[&str],
        query: &[(&str, &str)],
        timeout: Duration,
    ) -> Result<(), PluginProblem> {
        self.send(Method::DELETE, self.url(segments, query), None, timeout)
            .await
            .map(drop)
    }

    /// 同 [`Self::delete`]，错误带类别、原话和 `in_use` 的依赖方。
    pub(crate) async fn delete_detailed(
        &self,
        segments: &[&str],
        query: &[(&str, &str)],
        body: Option<&impl serde::Serialize>,
        timeout: Duration,
    ) -> Result<(), CoreApiError> {
        let body = body
            .map(serde_json::to_vec)
            .transpose()
            .map_err(|error| CoreApiError::Protocol(error.to_string()))?;
        let (status, bytes) = self
            .exchange(
                Method::DELETE,
                self.url(segments, query),
                body,
                "application/json",
                timeout,
            )
            .await
            .map_err(CoreApiError::Transport)?;
        if status >= 400 {
            return Err(http_error(status, &bytes));
        }
        Ok(())
    }

    /// `PluginImage.path` 是核心给的地址，只取图片接口下面的。
    pub(crate) async fn image(&self, image: &PluginImage) -> Result<Vec<u8>, PluginProblem> {
        if !image.path.starts_with(IMAGE_PREFIX) {
            return Err(PluginProblem {
                reason: PluginFailure::Invalid,
                min_version: None,
                message: l10n!(
                    CATALOG,
                    language(),
                    "plugins-api-not-image-url",
                    reason = image.path.as_str()
                ),
                settings: None,
            });
        }
        let url = self.base.join(&image.path).map_err(|e| PluginProblem {
            reason: PluginFailure::Invalid,
            min_version: None,
            message: l10n!(
                CATALOG,
                language(),
                "plugins-api-not-image-url",
                reason = e.to_string()
            ),
            settings: None,
        })?;
        // 上面看的是规范化之前的写法，`images/../devices` 也能通过。
        if !url.path().starts_with(IMAGE_PREFIX) {
            return Err(PluginProblem {
                reason: PluginFailure::Invalid,
                min_version: None,
                message: l10n!(
                    CATALOG,
                    language(),
                    "plugins-api-not-image-url",
                    reason = image.path.as_str()
                ),
                settings: None,
            });
        }
        self.send(Method::GET, url, None, READ_TIMEOUT).await
    }
}

fn parse<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, PluginProblem> {
    serde_json::from_slice(bytes).map_err(|e| PluginProblem {
        reason: PluginFailure::Protocol,
        min_version: None,
        message: l10n!(
            CATALOG,
            language(),
            "plugins-api-invalid-response",
            reason = e.to_string()
        ),
        settings: None,
    })
}

/// 原文只进日志：主人看到的是下一步怎么办，连接的情况由 `HomeViewModel` 说（ADR-0032）。
fn transport(error: &reqwest::Error, timeout: Duration) -> PluginProblem {
    if error.is_timeout() {
        tracing::info!("no response within {} seconds", timeout.as_secs());
    } else {
        tracing::info!("request failed: {error}");
    }
    not_connected()
}

pub(crate) fn not_connected() -> PluginProblem {
    PluginProblem {
        reason: PluginFailure::TransportUnavailable,
        min_version: None,
        message: l10n!(CATALOG, language(), "plugins-api-not-connected"),
        settings: None,
    }
}

/// 这台手机还没有配对信息：不是连不上，是还没配对，壳据此引导去配对。
pub(crate) fn not_paired() -> PluginProblem {
    PluginProblem {
        reason: PluginFailure::NotPaired,
        min_version: None,
        message: l10n!(CATALOG, language(), "plugins-api-not-paired"),
        settings: None,
    }
}
