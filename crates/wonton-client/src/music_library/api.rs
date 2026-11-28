//! OpenSubsonic 的请求层：经本地代理打插件网页下的 `/rest`，不带 `u`/`p`。
//!
//! 身份由核心网关按这台设备的人注入代登录头（插件清单 `webui.proxy.headers`），曲库按头认人，客户端不存上游口令。
//! Navidrome 0.64.2 实测：白名单内的来源带头时 `/rest` 不要 `u`/`p`；头里的人还没有账号时回错误码 40，
//! 带头打开一次网页（`GET /app/`）就把账号建出来。所以 40 先建号再重试一次，还是 40 才算没登录。
//!
//! 没用 `opensubsonic` 库：它每个请求都强制带 `u`/`t` 或 `apiKey`，没有只靠代理头认人的模式。

use std::time::Duration;

use reqwest::Client;
use serde::de::DeserializeOwned;
use url::Url;

const API_VERSION: &str = "1.16.1";
const CLIENT_NAME: &str = "wonton";
const TIMEOUT: Duration = Duration::from_secs(20);

/// Subsonic 的“用户名或密码不对”：代理头里的人在曲库里还没有账号。
pub(super) const CODE_WRONG_CREDENTIALS: i64 = 40;

/// 一台曲库：插件网页的地址和经本地代理的 HTTP 客户端。
#[derive(Clone)]
pub(crate) struct Endpoint {
    pub(crate) client: Client,
    /// 插件网页的根地址，以 `/` 结尾。
    pub(crate) base: Url,
}

/// 一次请求失败的原样原因，由调用方按插件状态分类成 [`wonton_protocol::music::MusicFailure`]。
#[derive(Debug)]
pub(super) enum Failure {
    /// 没拿到 HTTP 响应。
    Transport(String),
    /// 拿到了非 2xx。
    Status(u16),
    /// Subsonic 回了 `status: failed`。
    Subsonic { code: i64, message: String },
    /// 响应看不懂。
    Protocol(String),
}

impl Endpoint {
    fn url(&self, method: &str, params: &[(&str, &str)]) -> Result<Url, Failure> {
        let mut url = self
            .base
            .join(&format!("rest/{method}"))
            .map_err(|error| Failure::Protocol(error.to_string()))?;
        url.query_pairs_mut()
            .append_pair("v", API_VERSION)
            .append_pair("c", CLIENT_NAME)
            .append_pair("f", "json")
            .extend_pairs(params);
        Ok(url)
    }

    async fn get(&self, url: Url) -> Result<reqwest::Response, Failure> {
        let response = self
            .client
            .get(url)
            .timeout(TIMEOUT)
            .send()
            .await
            .map_err(|error| Failure::Transport(error.to_string()))?;
        if response.status().is_success() {
            Ok(response)
        } else {
            Err(Failure::Status(response.status().as_u16()))
        }
    }

    /// 调一个返回 JSON 的端点，取响应里 `key` 那一段；`key` 为空时只看成功与否。
    pub(super) async fn call<T: DeserializeOwned + Default>(
        &self,
        method: &str,
        params: &[(&str, &str)],
        key: &str,
    ) -> Result<T, Failure> {
        let bytes = self
            .get(self.url(method, params)?)
            .await?
            .bytes()
            .await
            .map_err(|error| Failure::Transport(error.to_string()))?;
        let mut response = envelope(&bytes)?;
        if key.is_empty() {
            return Ok(T::default());
        }
        match response.get_mut(key).map(serde_json::Value::take) {
            Some(value) => {
                serde_json::from_value(value).map_err(|error| Failure::Protocol(error.to_string()))
            }
            None => Err(Failure::Protocol(format!("no `{key}` in the response"))),
        }
    }

    /// 取二进制（封面）。出错时 Subsonic 回的是 JSON 错误体而不是图片，按内容类型认出来再拆。
    pub(super) async fn bytes(
        &self,
        method: &str,
        params: &[(&str, &str)],
    ) -> Result<Vec<u8>, Failure> {
        let response = self.get(self.url(method, params)?).await?;
        let is_json = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.contains("json"));
        let bytes = response
            .bytes()
            .await
            .map_err(|error| Failure::Transport(error.to_string()))?;
        if is_json {
            envelope(&bytes)?;
            return Err(Failure::Protocol(format!(
                "{method} returned JSON, not bytes"
            )));
        }
        Ok(bytes.to_vec())
    }

    /// 带代登录头打开一次网页，让曲库给这个人建账号。
    pub(super) async fn sign_in(&self) -> Result<(), Failure> {
        let url = self
            .base
            .join("app/")
            .map_err(|error| Failure::Protocol(error.to_string()))?;
        self.get(url).await.map(|_| ())
    }
}

/// 拆 `subsonic-response`：`status` 不是 `ok` 时回 [`Failure::Subsonic`]。
fn envelope(bytes: &[u8]) -> Result<serde_json::Value, Failure> {
    let mut body: serde_json::Value =
        serde_json::from_slice(bytes).map_err(|error| Failure::Protocol(error.to_string()))?;
    let response = body
        .get_mut("subsonic-response")
        .map(serde_json::Value::take)
        .ok_or_else(|| Failure::Protocol("no subsonic-response".into()))?;
    if response.get("status").and_then(|s| s.as_str()) == Some("ok") {
        return Ok(response);
    }
    let error = response.get("error");
    Err(Failure::Subsonic {
        code: error
            .and_then(|e| e.get("code"))
            .and_then(serde_json::Value::as_i64)
            .unwrap_or(0),
        message: error
            .and_then(|e| e.get("message"))
            .and_then(|m| m.as_str())
            .unwrap_or_default()
            .to_owned(),
    })
}
