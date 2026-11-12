//! 核心的插件管理接口（ADR-0017 的“管理接口”）：请求走本地代理，核心凭隧道身份认出这台已配对设备。请求不带
//! `Origin` 和 `Sec-Fetch-Site`，核心据此知道它不是网页发起的。

#[path = "api_error.rs"]
mod error;
use error::core_error;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use reqwest::header::{ACCEPT_LANGUAGE, CONTENT_TYPE};
use reqwest::{Client, Method};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use url::Url;
use wonton_l10n::l10n;
use wonton_protocol::{
    CoreError, CoreUpgradeFailure, CoreUpgradeProblem, CoreUpgradeStatus, PluginFailure,
    PluginImage, PluginProblem, SettingsFieldError, SettingsSchemaProblem, minimum_server_version,
    window_path,
};

use crate::chat::proxy_url;
use crate::l10n::{CATALOG, language};
use crate::plugins::lock;

/// 列表、详情、图片：只读家里服务器上已有的东西。隧道卡住时按“连不上”失败，不一直转圈。
pub(crate) const READ_TIMEOUT: Duration = Duration::from_secs(30);
/// 问服务器版本：接口已经回过 404，等久了不如先按原来的类别报。
const VERSION_TIMEOUT: Duration = Duration::from_secs(5);
/// 家里的服务器要去来源（GitHub）取东西：商店、商店里的详情、预览，以及引导要取的官方索引和图标。
pub(crate) const SOURCE_TIMEOUT: Duration = Duration::from_secs(120);
/// 安装和升级：响应要等拉完镜像、通过健康检查。Hermes 的镜像约 1 GB，慢的网络要很久；超时后核心照样做完，
/// 结果从已安装列表里看。核心等健康至多 60 分钟（按插件的 healthcheck.start_period 放宽），再留出拉镜像的时间。
pub(super) const INSTALL_TIMEOUT: Duration = Duration::from_secs(90 * 60);

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
    #[serde(default)]
    problem: Option<String>,
    #[serde(default)]
    fields: Vec<SettingsFieldError>,
    #[serde(default)]
    problems: Vec<SettingsSchemaProblem>,
}
pub(crate) enum CoreApiError {
    Transport(PluginProblem),
    Http {
        status: u16,
        kind: Option<String>,
        message: String,
        problem: Option<String>,
    },
    Protocol(String),
}

/// 一次请求要用的连接信息：核心的地址、本地代理，以及主人的语言。
#[derive(Clone)]
pub(crate) struct CoreApi {
    pub(super) base: Url,
    pub(super) client: Client,
    pub(super) languages: String,
    /// 这台服务器报过的版本。只有接口回 404 时才去取一次，取不到就不判旧。
    pub(super) version: ServerVersion,
}

/// 服务器版本的一次性缓存，随作用域活着：升级状态的每次成功回应都写进来。
#[derive(Clone, Default)]
pub(crate) struct ServerVersion(Arc<VersionCache>);

#[derive(Default)]
struct VersionCache {
    value: Mutex<Option<String>>,
    /// 问过一次了：服务器不报版本时不再每次都问。
    asked: AtomicBool,
}

impl ServerVersion {
    /// 记下服务器自己报的版本；空的不覆盖已有的。
    pub(super) fn seed(&self, version: Option<String>) {
        if let Some(version) = version {
            *lock(&self.0.value) = Some(version);
        }
    }

    fn known(&self) -> Option<String> {
        lock(&self.0.value).clone()
    }

    /// 还不知道时问服务器一次（核心升级状态的 `current.version`）；问不到就是不知道。
    async fn get(&self, api: &CoreApi) -> Option<String> {
        if let Some(version) = self.known() {
            return Some(version);
        }
        if self.0.asked.swap(true, Ordering::SeqCst) {
            return None;
        }
        #[derive(Deserialize)]
        struct Reported {
            current: Current,
        }
        #[derive(Deserialize)]
        struct Current {
            #[serde(default)]
            version: Option<String>,
        }
        let probed = match api
            .exchange(
                Method::GET,
                api.url(&["core", "upgrade"], &[]),
                None,
                "application/json",
                VERSION_TIMEOUT,
            )
            .await
        {
            Ok((status, bytes)) if status < 400 => serde_json::from_slice::<Reported>(&bytes)
                .ok()
                .and_then(|reported| reported.current.version),
            _ => None,
        };
        self.seed(probed.clone());
        probed
    }
}

/// 建本地代理的 HTTP 客户端。
pub(super) fn proxied_client(
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
    fn url(&self, segments: &[&str], query: &[(&str, &str)]) -> Url {
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

    async fn exchange(
        &self,
        method: Method,
        url: Url,
        body: Option<Vec<u8>>,
        content_type: &str,
        timeout: Duration,
    ) -> Result<(u16, Vec<u8>), PluginProblem> {
        let mut request = self.client.request(method, url).timeout(timeout);
        if !self.languages.is_empty() {
            request = request.header(ACCEPT_LANGUAGE, &self.languages);
        }
        if let Some(body) = body {
            request = request.header(CONTENT_TYPE, content_type).body(body);
        }
        let response = request.send().await.map_err(|e| transport(&e, timeout))?;
        let status = response.status().as_u16();
        let bytes = response.bytes().await.map_err(|e| transport(&e, timeout))?;
        Ok((status, bytes.to_vec()))
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
    pub(super) async fn core_upgrade(
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
    pub(super) async fn post_archive<T: DeserializeOwned>(
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
    pub(crate) async fn put_unit_detailed(
        &self,
        segments: &[&str],
        body: &impl serde::Serialize,
        timeout: Duration,
    ) -> Result<(), CoreApiError> {
        self.unit_detailed(Method::PUT, segments, body, timeout)
            .await
    }

    /// 发 JSON 请求体，不要响应体，错误带类别和消息：测试模型服务商要分得清 key 被拒和连不上。
    pub(crate) async fn post_unit_detailed(
        &self,
        segments: &[&str],
        body: &impl serde::Serialize,
        timeout: Duration,
    ) -> Result<(), CoreApiError> {
        self.unit_detailed(Method::POST, segments, body, timeout)
            .await
    }

    async fn unit_detailed(
        &self,
        method: Method,
        segments: &[&str],
        body: &impl serde::Serialize,
        timeout: Duration,
    ) -> Result<(), CoreApiError> {
        let body = serde_json::to_vec(body).map_err(|error| {
            CoreApiError::Protocol(l10n!(
                CATALOG,
                language(),
                "plugins-api-invalid-request",
                reason = error.to_string()
            ))
        })?;
        let (status, bytes) = self
            .exchange(
                method,
                self.url(segments, &[]),
                Some(body),
                "application/json",
                timeout,
            )
            .await
            .map_err(CoreApiError::Transport)?;
        if status < 400 {
            return Ok(());
        }
        Err(http_error(status, &bytes))
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
    pub(super) async fn empty<T: DeserializeOwned>(
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
    pub(super) async fn stop_window(
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
    pub(super) async fn post_unit(
        &self,
        segments: &[&str],
        body: &impl serde::Serialize,
        timeout: Duration,
    ) -> Result<(), PluginProblem> {
        self.post_raw(segments, body, timeout).await.map(drop)
    }

    pub(super) async fn post_raw(
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

    /// 响应是空的。
    pub(super) async fn put_unit(
        &self,
        segments: &[&str],
        timeout: Duration,
    ) -> Result<(), PluginProblem> {
        self.send(Method::PUT, self.url(segments, &[]), None, timeout)
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

/// 错误响应里能认出来的类别和原话；认不出来时只留状态码。
fn http_error(status: u16, bytes: &[u8]) -> CoreApiError {
    let detail = serde_json::from_slice::<ErrorBody>(bytes)
        .ok()
        .map(|body| body.error);
    let (kind, message, problem) = match detail {
        Some(ErrorDetail {
            kind,
            message,
            problem,
            ..
        }) => (Some(kind), message, problem),
        None => (
            None,
            l10n!(
                CATALOG,
                language(),
                "plugins-api-server-status",
                status = status
            ),
            None,
        ),
    };
    CoreApiError::Http {
        status,
        kind,
        message,
        problem,
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

pub(super) fn not_connected() -> PluginProblem {
    PluginProblem {
        reason: PluginFailure::TransportUnavailable,
        min_version: None,
        message: l10n!(CATALOG, language(), "plugins-api-not-connected"),
        settings: None,
    }
}

fn upgrade_error(status: u16, body: &[u8]) -> CoreUpgradeProblem {
    let detail = serde_json::from_slice::<ErrorBody>(body)
        .ok()
        .map(|body| body.error);
    let reason = match detail.as_ref().map(|detail| detail.kind.as_str()) {
        Some("unauthenticated" | "cross_site") => CoreUpgradeFailure::Unauthorized,
        Some("changed") => CoreUpgradeFailure::Changed,
        Some("not_ready") => CoreUpgradeFailure::NotReady,
        Some("busy") => CoreUpgradeFailure::Busy,
        Some("too_late") => CoreUpgradeFailure::TooLate,
        Some(_) if status >= 500 => CoreUpgradeFailure::Upstream,
        Some(_) => CoreUpgradeFailure::Protocol,
        None => match status {
            401 | 403 => CoreUpgradeFailure::Unauthorized,
            502 => CoreUpgradeFailure::TransportUnavailable,
            500..=599 => CoreUpgradeFailure::Upstream,
            _ => CoreUpgradeFailure::Protocol,
        },
    };
    let message = detail.map_or_else(
        || {
            l10n!(
                CATALOG,
                language(),
                "plugins-api-server-status",
                status = status
            )
        },
        |detail| detail.message,
    );
    CoreUpgradeProblem { reason, message }
}

/// 直接返回结果的调用（详情、预览、图片）按 [`CoreError`] 的分类失败，核心写的说明原样带上。
pub(crate) fn to_core_error(problem: PluginProblem) -> CoreError {
    let reason = problem.message;
    match problem.reason {
        PluginFailure::TransportUnavailable => CoreError::TransportUnavailable { reason },
        PluginFailure::Unauthorized => CoreError::Unauthorized { reason },
        PluginFailure::SettingsInvalid => CoreError::SettingsInvalid {
            reason,
            fields: problem.settings.map(|settings| settings.fields).unwrap_or_default(),
        },
        PluginFailure::NotFound
        | PluginFailure::Invalid
        | PluginFailure::Changed
        | PluginFailure::SettingsSchemaInvalid => CoreError::InvalidInput { reason },
        PluginFailure::Unsupported
        | PluginFailure::Conflict
        | PluginFailure::DeviceUnavailable
        | PluginFailure::LanUnavailable
        | PluginFailure::LanPortTaken
        | PluginFailure::EgressProxyUnavailable
        | PluginFailure::CoreUpgrading
        // 记录不可用：现在做不了。壳按类别出「重新安装」的下一步（#198 的核心行为、#230 的 iOS 壳）。
        | PluginFailure::RecordUnusable => CoreError::NotReady { reason },
        PluginFailure::SourceUnavailable | PluginFailure::Upstream | PluginFailure::Protocol => {
            CoreError::Upstream { reason }
        }
        PluginFailure::ServerTooOld => CoreError::ServerTooOld {
            reason,
            min_version: problem.min_version,
        },
    }
}
