//! 扫码配对的线协议，规则见 ADR-0009 的“接口：扫码配对”。
//!
//! - 二维码里是一个 [`PairingCode`]：`wonton://pair?…`，带服务端的地址、基础域名、核心地址和一次性的配对密钥。
//! - 还没登记的设备用 [`PAIR_ALPN`] 连上服务端，开一条双向流，写入 JSON 的 [`PairRequest`] 后结束发送；服务端回
//!   JSON 的 [`PairResponse`] 后结束。消息都不超过 [`MAX_MESSAGE_LEN`] 字节。配对成功时，服务端已经按 QUIC 握手
//!   认证过的 endpoint ID 登记了这台设备，设备随后改用隧道的 ALPN 连接。

mod client;
mod server;

use std::fmt;
use std::net::SocketAddr;
use std::str::FromStr;

use iroh::{EndpointAddr, EndpointId};
use serde::{Deserialize, Serialize};
use url::Url;

pub use client::{PAIR_TIMEOUT, pair};
pub use server::{PairHandler, PairProtocol};

use crate::relays::Relay;

/// 配对协议的 ALPN，和隧道的 ALPN 挂在同一个 endpoint 上。协议不兼容地变化时升级末尾的版本号。
pub const PAIR_ALPN: &[u8] = b"wonton/pair/0";

/// 请求和响应的长度上限：服务端读到这么多还没结束就断开，未登记的设备占不了更多内存。
pub const MAX_MESSAGE_LEN: usize = 4096;

/// 二维码里的配对码版本。格式不兼容地变化时加一，旧 App 认出来后提示更新，而不是当成坏码。
const CODE_VERSION: &str = "0";

/// 配对密钥的字节数。128 位随机数猜不中，服务端不用为它限速。
pub const SECRET_LEN: usize = 16;

/// 二维码里的一次性配对码。
#[derive(Clone, PartialEq, Eq)]
pub struct PairingCode {
    pub server_id: EndpointId,
    /// 服务端的中继表：地址、可选准入令牌、档位（ADR-0048）。
    pub relays: Vec<Relay>,
    /// 手机能直接到达的地址，例如局域网 IP 加核心发布的隧道端口。至少要有一个中继地址或直连地址：
    /// n0 的地址发现在国内靠不住（ADR-0004）。
    pub direct_addrs: Vec<SocketAddr>,
    /// 本地代理只转发这个域名和它的子域名（ADR-0004）。小写，不带末尾的点。
    pub base_domain: String,
    /// 核心自己的地址（经隧道访问），协议和端口由核心的公开入口决定。主机名就是基础域名。
    pub core_url: Url,
    pub secret: PairingSecret,
}

/// 配对密钥：[`SECRET_LEN`] 字节，十六进制小写。`Debug` 不打印原文。
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PairingSecret(String);

impl PairingSecret {
    /// 用给定的随机字节组成密钥。
    pub fn from_bytes(bytes: [u8; SECRET_LEN]) -> Self {
        Self(bytes.iter().map(|b| format!("{b:02x}")).collect())
    }

    pub fn expose(&self) -> &str {
        &self.0
    }

    /// 从二维码或请求里的十六进制文字读回密钥，格式不对时为空。
    pub fn from_hex(text: &str) -> Option<Self> {
        let valid = text.len() == SECRET_LEN * 2
            && text
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
        valid.then(|| Self(text.to_owned()))
    }
}

impl fmt::Debug for PairingSecret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<redacted>")
    }
}

impl fmt::Debug for PairingCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PairingCode")
            .field("server_id", &self.server_id.fmt_short().to_string())
            .field("relays", &self.relays)
            .field("direct_addrs", &self.direct_addrs)
            .field("base_domain", &self.base_domain)
            .field("core_url", &self.core_url.as_str())
            .field("secret", &self.secret)
            .finish()
    }
}

/// 扫到的不是能用的配对码。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum InvalidCode {
    #[error("not a Wonton pairing code")]
    NotPairingCode,
    /// 配对码比这个 App 新：要更新 App，换一个码没有用。
    #[error("pairing code format version {0} is not recognized by this app; update the app")]
    UnsupportedVersion(String),
    #[error("invalid {field} in the pairing code: {reason}")]
    Malformed { field: &'static str, reason: String },
}

impl PairingCode {
    /// 解析二维码里的文字。
    pub fn parse(text: &str) -> Result<Self, InvalidCode> {
        let url = Url::parse(text.trim()).map_err(|_| InvalidCode::NotPairingCode)?;
        if url.scheme() != "wonton" || url.host_str() != Some("pair") {
            return Err(InvalidCode::NotPairingCode);
        }
        let mut fields = Fields::default();
        for (key, value) in url.query_pairs() {
            fields.push(&key, value.into_owned());
        }
        match fields.version.as_deref() {
            Some(CODE_VERSION) => {}
            Some(other) => return Err(InvalidCode::UnsupportedVersion(other.to_owned())),
            None => return Err(malformed("version", "missing v")),
        }
        fields.build()
    }

    /// 写成二维码里的文字。
    pub fn to_uri(&self) -> String {
        let mut url = Url::parse("wonton://pair").expect("the fixed URL always parses");
        {
            let mut query = url.query_pairs_mut();
            query.append_pair("v", CODE_VERSION);
            query.append_pair("id", &self.server_id.to_string());
            // 整张中继表（连令牌和档位）放在一个参数里，和 `/_wonton/api/tunnel` 的 `relays` 同一个写法。
            let relays = serde_json::to_string(&self.relays)
                .expect("the relay table always serializes to JSON");
            query.append_pair("relays", &relays);
            for addr in &self.direct_addrs {
                query.append_pair("addr", &addr.to_string());
            }
            query.append_pair("domain", &self.base_domain);
            query.append_pair("core", self.core_url.as_str());
            query.append_pair("secret", self.secret.expose());
        }
        url.into()
    }

    /// 连服务端用的地址。
    pub fn server_addr(&self) -> EndpointAddr {
        let mut addr = EndpointAddr::new(self.server_id);
        for relay in &self.relays {
            addr = addr.with_relay_url(relay.url.clone());
        }
        for ip in &self.direct_addrs {
            addr = addr.with_ip_addr(*ip);
        }
        addr
    }
}

#[derive(Default)]
struct Fields {
    version: Option<String>,
    id: Option<String>,
    relays: Option<String>,
    addrs: Vec<String>,
    domain: Option<String>,
    core: Option<String>,
    secret: Option<String>,
}

impl Fields {
    /// 不认识的参数忽略，同一个版本里可以往码里加可选的字段。
    fn push(&mut self, key: &str, value: String) {
        match key {
            "v" => self.version = Some(value),
            "id" => self.id = Some(value),
            "relays" => self.relays = Some(value),
            "addr" => self.addrs.push(value),
            "domain" => self.domain = Some(value),
            "core" => self.core = Some(value),
            "secret" => self.secret = Some(value),
            _ => {}
        }
    }

    fn build(self) -> Result<PairingCode, InvalidCode> {
        let server_id = self
            .id
            .as_deref()
            .ok_or_else(|| malformed("server ID", "missing id"))
            .and_then(|id| {
                EndpointId::from_str(id).map_err(|e| malformed("server ID", &e.to_string()))
            })?;
        let relays = match self.relays.as_deref() {
            Some(text) => serde_json::from_str::<Vec<Relay>>(text)
                .map_err(|e| malformed("relay table", &e.to_string()))?,
            None => Vec::new(),
        };
        for relay in &relays {
            relay
                .validate()
                .map_err(|reason| malformed("relay table", &reason))?;
        }
        let direct_addrs = self
            .addrs
            .iter()
            .map(|addr| {
                SocketAddr::from_str(addr).map_err(|e| malformed("direct address", &e.to_string()))
            })
            .collect::<Result<Vec<_>, _>>()?;
        if relays.is_empty() && direct_addrs.is_empty() {
            return Err(malformed(
                "server address",
                "no relay URL or direct address",
            ));
        }
        let base_domain = base_domain(self.domain.as_deref())?;
        let core_url = core_url(self.core.as_deref(), &base_domain)?;
        let secret = self
            .secret
            .as_deref()
            .and_then(PairingSecret::from_hex)
            .ok_or_else(|| malformed("pairing secret", "expected 32 lowercase hex digits"))?;
        Ok(PairingCode {
            server_id,
            relays,
            direct_addrs,
            base_domain,
            core_url,
            secret,
        })
    }
}

fn base_domain(text: Option<&str>) -> Result<String, InvalidCode> {
    let text = text.ok_or_else(|| malformed("base domain", "missing domain"))?;
    let domain = text.trim_end_matches('.').to_ascii_lowercase();
    let valid_label = |label: &str| {
        !label.is_empty()
            && label
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-')
    };
    if domain.split('.').all(valid_label) {
        Ok(domain)
    } else {
        Err(malformed("base domain", text))
    }
}

fn core_url(text: Option<&str>, domain: &str) -> Result<Url, InvalidCode> {
    let text = text.ok_or_else(|| malformed("core URL", "missing core"))?;
    let url = Url::parse(text).map_err(|e| malformed("core URL", &e.to_string()))?;
    let host = url.host_str().unwrap_or_default().to_ascii_lowercase();
    if matches!(url.scheme(), "http" | "https") && host == domain {
        Ok(url)
    } else {
        Err(malformed(
            "core URL",
            &format!("must be an http(s) URL under {domain}"),
        ))
    }
}

fn malformed(field: &'static str, reason: &str) -> InvalidCode {
    InvalidCode::Malformed {
        field,
        reason: reason.to_owned(),
    }
}

/// 设备经 [`PAIR_ALPN`] 发给服务端的请求。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PairRequest {
    /// 出示配对密钥，请服务端把本设备登记成主人的设备。
    Pair {
        secret: PairingSecret,
        /// 主人在设备列表里认它用，如“Alice 的 iPhone”。
        device_name: String,
    },
}

/// 服务端的回应。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "result", rename_all = "snake_case")]
pub enum PairResponse {
    /// 已登记，可以用隧道的 ALPN 连接了。
    Paired,
    Rejected {
        reason: RejectReason,
        /// 给人看的说明。
        message: String,
    },
}

/// 服务端拒绝配对的原因。App 按它提示主人该做什么。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RejectReason {
    /// 服务端没有发过这个码。
    UnknownCode,
    /// 码过期了，要换一个新码。
    Expired,
    /// 码已经用过。
    Used,
    /// 这种码已经不能再用来配对，例如服务器上已经有配对的设备，首次配对码随之作废。
    Closed,
    /// 请求不合规（格式不对、太长）。
    BadRequest,
    /// 服务端自己出错了（例如设备登记表写不进去），稍后再试。
    ServerError,
    /// 比这个 App 新的服务端给出的原因。
    #[serde(other)]
    Other,
}

#[cfg(test)]
mod tests {
    use iroh::SecretKey;

    use super::*;

    fn code() -> PairingCode {
        PairingCode {
            server_id: SecretKey::generate().public(),
            relays: vec![crate::relays::Relay {
                url: "https://relay.example.com./".parse().unwrap(),
                token: Some(crate::relays::RelayToken::new("tok")),
                tier: 0,
            }],
            direct_addrs: vec![
                "192.168.1.20:18482".parse().unwrap(),
                "[fd00::20]:18482".parse().unwrap(),
            ],
            base_domain: "wonton.localhost".to_owned(),
            core_url: Url::parse("http://wonton.localhost:18480/").unwrap(),
            secret: PairingSecret::from_bytes([0xab; SECRET_LEN]),
        }
    }

    #[test]
    fn a_code_survives_its_own_uri() {
        let code = code();
        let uri = code.to_uri();
        assert!(uri.starts_with("wonton://pair?v=0&"), "{uri}");
        assert_eq!(PairingCode::parse(&uri).unwrap(), code);
        // 二维码扫出来的文字常带首尾空白。
        assert_eq!(PairingCode::parse(&format!(" {uri}\n")).unwrap(), code);
    }

    #[test]
    fn other_qr_codes_and_newer_formats_are_told_apart() {
        for text in ["https://example.com/", "hello", "wonton://chat?v=0"] {
            assert_eq!(
                PairingCode::parse(text),
                Err(InvalidCode::NotPairingCode),
                "{text}"
            );
        }
        let newer = code().to_uri().replace("v=0", "v=1");
        assert_eq!(
            PairingCode::parse(&newer),
            Err(InvalidCode::UnsupportedVersion("1".to_owned()))
        );
    }

    #[test]
    fn a_code_needs_an_address_a_valid_secret_and_a_core_under_the_domain() {
        let uri = code().to_uri();
        let without = |param: &str| {
            let mut url = Url::parse(&uri).unwrap();
            let pairs: Vec<(String, String)> = url
                .query_pairs()
                .filter(|(key, _)| !param.split(',').any(|p| p == key))
                .map(|(k, v)| (k.into_owned(), v.into_owned()))
                .collect();
            url.query_pairs_mut().clear().extend_pairs(pairs);
            url.to_string()
        };
        for broken in [
            without("relays,addr"),
            without("secret"),
            without("core"),
            uri.replace("secret=abab", "secret=ABAB"),
            uri.replace("wonton.localhost%3A18480", "evil.example%3A18480"),
        ] {
            assert!(
                matches!(
                    PairingCode::parse(&broken),
                    Err(InvalidCode::Malformed { .. })
                ),
                "{broken}"
            );
        }
    }

    #[test]
    fn debug_hides_the_secret() {
        let code = code();
        let debug = format!("{code:?}");
        assert!(!debug.contains(code.secret.expose()), "{debug}");
        let request = PairRequest::Pair {
            secret: code.secret.clone(),
            device_name: "phone".to_owned(),
        };
        assert!(!format!("{request:?}").contains(code.secret.expose()));
    }

    /// 两端各自编译，消息的 JSON 形状就是契约。
    #[test]
    fn messages_have_the_documented_json_shape() {
        let request = PairRequest::Pair {
            secret: PairingSecret::from_bytes([1; SECRET_LEN]),
            device_name: "Alice 的 iPhone".to_owned(),
        };
        assert_eq!(
            serde_json::to_value(&request).unwrap(),
            serde_json::json!({
                "type": "pair",
                "secret": "01010101010101010101010101010101",
                "device_name": "Alice 的 iPhone",
            })
        );
        assert_eq!(
            serde_json::to_value(PairResponse::Paired).unwrap(),
            serde_json::json!({"result": "paired"})
        );
        let rejected: PairResponse = serde_json::from_value(serde_json::json!({
            "result": "rejected", "reason": "expired", "message": "码过期了",
        }))
        .unwrap();
        assert_eq!(
            rejected,
            PairResponse::Rejected {
                reason: RejectReason::Expired,
                message: "码过期了".to_owned(),
            }
        );
        let newer: PairResponse = serde_json::from_value(serde_json::json!({
            "result": "rejected", "reason": "something_new", "message": "…",
        }))
        .unwrap();
        assert!(matches!(
            newer,
            PairResponse::Rejected {
                reason: RejectReason::Other,
                ..
            }
        ));
    }
}
