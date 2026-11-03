//! 按 Web Push 发推送（RFC 8030），内容按 RFC 8291 加密给设备，推送服务只看到密文。

use std::time::Duration;

use hyper::Uri;
use hyper::header::HeaderValue;
use web_push_native::p256::PublicKey;
use web_push_native::{Auth, WebPushBuilder};

use super::PushError;
use super::store::Subscription;
use crate::plugin::egress::core_proxy::CoreProxy;

/// 推送服务替离线的设备留多久（RFC 8030 的 `TTL`）。隔了一天的新消息通知意义不大，App 打开时会同步到。
const TTL: Duration = Duration::from_secs(24 * 60 * 60);

/// 一次推送最多等多久。Tuwunel 等推送代理回话，推送服务卡住时不能一直占着它。
const TIMEOUT: Duration = Duration::from_secs(10);

/// 推送服务拒绝时，它的说明最多记这么长。
const MAX_REASON: usize = 300;

/// 推送服务怎么处理了这条推送。暂时不可用（连不上、5xx、429）的情况是 [`PushError::Unavailable`]。
#[derive(Debug, PartialEq, Eq)]
pub enum Delivery {
    Accepted,
    /// 订阅已失效（404、410）：设备换了推送地址，或者卸载了 App。
    Gone,
    /// 推送服务拒绝了这条推送（其他 4xx），重试也没用。
    Refused {
        status: u16,
        reason: String,
    },
}

pub struct WebPush {
    client: reqwest::Client,
}

impl WebPush {
    pub fn new(proxy: &CoreProxy) -> Result<Self, PushError> {
        let client = reqwest::Client::builder()
            .proxy(proxy.for_client())
            .timeout(TIMEOUT)
            // 推送地址来自设备，不跟随跳转，免得核心被引去请求别的地址。
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|e| PushError::Client(e.to_string()))?;
        Ok(Self { client })
    }

    pub async fn send(
        &self,
        subscription: &Subscription,
        payload: Vec<u8>,
        urgency: &'static str,
    ) -> Result<Delivery, PushError> {
        let endpoint: Uri = subscription
            .endpoint
            .parse()
            .map_err(|e| PushError::Invalid(format!("推送地址不合法：{e}")))?;
        let key = PublicKey::from_sec1_bytes(&subscription.public_key())
            .map_err(|_| PushError::Invalid("p256dh 不是 P-256 曲线上的点".to_owned()))?;
        let auth = Auth::clone_from_slice(&subscription.auth_secret());
        let mut request = WebPushBuilder::new(endpoint, key, auth)
            .with_valid_duration(TTL)
            .build(payload)
            .map_err(|e| PushError::Invalid(format!("加密推送失败：{e}")))?;
        request
            .headers_mut()
            .insert("urgency", HeaderValue::from_static(urgency));
        let request = reqwest::Request::try_from(request)
            .map_err(|e| PushError::Invalid(format!("推送请求不合法：{e}")))?;
        let host = subscription.endpoint_host();
        let response = self.client.execute(request).await.map_err(|e| {
            PushError::Unavailable(format!(
                "连不上推送服务 {host}：{}",
                crate::http::error_chain(&e)
            ))
        })?;
        let status = response.status();
        if status.is_success() {
            return Ok(Delivery::Accepted);
        }
        match status.as_u16() {
            404 | 410 => Ok(Delivery::Gone),
            429 | 500..=599 => Err(PushError::Unavailable(format!(
                "推送服务 {host} 回了 {status}"
            ))),
            code => {
                let text = response.text().await.unwrap_or_default();
                Ok(Delivery::Refused {
                    status: code,
                    reason: text.chars().take(MAX_REASON).collect(),
                })
            }
        }
    }
}
