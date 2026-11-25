//! 发往我们运营的中继时的服务器身份（RFC 8292 的 VAPID）：私钥给每条推送签一个短 JWT，中继按公钥认出
//! 是哪台服务器的请求并按它限速。
//!
//! 中继只转发，不存令牌：设备拿到的推送地址里带着 APNs 令牌，中继见到密文和令牌就够发。私钥是不带填充的
//! base64url（32 字节 P-256 标量），公钥在中继的 `servers.d` 里登记（见 `cloud/push-relay`）。

use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use hyper::Uri;
use hyper::header::HeaderValue;
use web_push_native::jwt_simple::algorithms::ES256KeyPair;
use web_push_native::vapid::VapidSignature;

#[derive(Debug)]
pub struct Vapid {
    key: ES256KeyPair,
    subject: String,
}

impl Vapid {
    pub fn from_key(text: &str, subject: String) -> Result<Self, String> {
        let bytes = URL_SAFE_NO_PAD
            .decode(text.trim())
            .map_err(|e| format!("私钥不是 base64url：{e}"))?;
        // `ES256KeyPair::from_bytes` 对长度不对的输入会 panic，先自己挡。
        if bytes.len() != 32 {
            return Err(format!("私钥要 32 字节，读到 {} 字节", bytes.len()));
        }
        let key =
            ES256KeyPair::from_bytes(&bytes).map_err(|e| format!("私钥不是 P-256 标量：{e}"))?;
        Ok(Self { key, subject })
    }

    /// `aud` 由推送地址的 scheme 和 host 决定，中继会核对它必须是自己的地址。
    pub fn header(&self, endpoint: &Uri, valid: Duration) -> Result<HeaderValue, String> {
        VapidSignature::sign(endpoint, valid, &self.subject, &self.key)
            .map(Into::into)
            .map_err(|e| format!("签 VAPID 失败：{e}"))
    }
}

#[cfg(test)]
#[path = "vapid_tests.rs"]
mod vapid_tests;
