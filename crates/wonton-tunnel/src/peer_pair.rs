//! 服务器配对（ADR-0050）：一台服务器的主人出码，另一台的主人扫码或粘贴，出码方的主人点同意后，
//! 两台服务器的 Matrix 联邦才经 iroh 互通。线协议的形状照设备的扫码配对（[`crate::pair`]）。
//!
//! - 二维码里是一个 [`PeerPairingCode`]：`wonton://peer?…`，带出码方的 endpoint ID、中继地址、Matrix
//!   服务器名和长期有效的请求令牌。不带直连地址：码会发到群里，带了就等于把本机的公网地址给了每个看到的人。
//! - 扫码方经 [`PEER_PAIR_ALPN`] 连出码方，发 JSON 的 [`PeerMessage::Scan`]（出示令牌、自己的服务器名、
//!   主人的用户 ID 和显示名、能被回连的地址）；出码方回 JSON 的 [`PeerReply`]：等主人同意（附自己的同样几项），
//!   或者拒绝的原因。
//! - 请求没有过期时间，所以扫码方自己来问结果：[`PeerMessage::Outcome`] 回等待中、已同意、已拒绝或不认识。
//! - 出码方的主人点了同意或拒绝后，出码方也连回扫码方（地址来自扫码请求），发 JSON 的
//!   [`PeerMessage::Decision`]；扫码方只接受它扫过的那个 endpoint ID（[`PeerPairHandler::expecting_decision`]
//!   说了算）发来的结果。任何一方删配对时发 [`PeerMessage::Removed`]，尽力而为，对方离线时靠联邦握手被拒兜底。

use std::fmt;
use std::net::SocketAddr;
use std::str::FromStr;

use iroh::{EndpointAddr, EndpointId, RelayUrl};
use serde::{Deserialize, Serialize};
use url::Url;

pub(crate) mod client;
pub(crate) mod server;

pub use client::{ask_outcome, deliver_decision, notify_removed, scan, scan_to};
pub use server::{PeerPairHandler, PeerPairProtocol};

/// 服务器配对协议的 ALPN，和隧道、设备配对、联邦挂在同一个 endpoint 上。协议不兼容地变化时升级末尾的版本号。
pub const PEER_PAIR_ALPN: &[u8] = b"wonton/peer/1";

/// 请求、回应和通知的长度上限，和设备配对一致：多余的字节占不住内存。
pub const MAX_MESSAGE_LEN: usize = 4096;

/// 二维码里的配对码版本。格式不兼容地变化时加一，旧 App 认出来后提示更新，而不是当成坏码。
const CODE_VERSION: &str = "1";

/// 请求令牌的字节数。128 位随机数猜不中，服务端不用为它限速。
pub const TOKEN_LEN: usize = 16;

/// 出码方的配对码，一直有效到主人重置。
#[derive(Clone, PartialEq, Eq)]
pub struct PeerPairingCode {
    /// 出码方的 endpoint ID，也是之后联邦和对端认定的身份。服务器名和它的绑定只在配对时定下（ADR-0050）。
    pub server_id: EndpointId,
    /// 出码方的中继地址。码里不带直连地址，对方经中继连上后由 iroh 自己找直连的路。
    pub relay_urls: Vec<RelayUrl>,
    /// 出码方的 Matrix 服务器名，永久不变（ADR-0009 的“补充：Matrix 的服务器名”）。
    pub server_name: String,
    /// 出码方的请求令牌：只说明对方拿到过这张码，把关的是主人点同意。
    pub token: PeerPairToken,
}

impl fmt::Debug for PeerPairingCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PeerPairingCode")
            .field("server_id", &self.server_id.fmt_short().to_string())
            .field("relay_urls", &self.relay_urls)
            .field("server_name", &self.server_name)
            .field("token", &self.token)
            .finish()
    }
}

/// 扫到的不是能用的服务器配对码。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum InvalidPeerCode {
    #[error("不是云吞的服务器配对码")]
    NotPeerCode,
    /// 配对码比这个核心新：要更新核心，换一个码没有用。
    #[error("配对码的格式版本是 {0}，这个核心认不出，请更新核心")]
    UnsupportedVersion(String),
    #[error("配对码里的{field}不对：{reason}")]
    Malformed { field: &'static str, reason: String },
}

impl PeerPairingCode {
    /// 解析二维码里的文字。
    pub fn parse(text: &str) -> Result<Self, InvalidPeerCode> {
        let url = Url::parse(text.trim()).map_err(|_| InvalidPeerCode::NotPeerCode)?;
        if url.scheme() != "wonton" || url.host_str() != Some("peer") {
            return Err(InvalidPeerCode::NotPeerCode);
        }
        let mut fields = Fields::default();
        for (key, value) in url.query_pairs() {
            fields.push(&key, value.into_owned());
        }
        match fields.version.as_deref() {
            Some(CODE_VERSION) => {}
            Some(other) => return Err(InvalidPeerCode::UnsupportedVersion(other.to_owned())),
            None => return Err(malformed("版本", "缺少 v")),
        }
        fields.build()
    }

    /// 写成二维码里的文字。
    pub fn to_uri(&self) -> String {
        let mut url = Url::parse("wonton://peer").expect("固定的地址总能解析");
        {
            let mut query = url.query_pairs_mut();
            query.append_pair("v", CODE_VERSION);
            query.append_pair("id", &self.server_id.to_string());
            for relay in &self.relay_urls {
                query.append_pair("relay", relay.as_str());
            }
            query.append_pair("name", &self.server_name);
            query.append_pair("token", self.token.expose());
        }
        url.into()
    }

    /// 连出码方用的地址：只有中继，直连地址由 iroh 自己找。
    pub fn server_addr(&self) -> EndpointAddr {
        let mut addr = EndpointAddr::new(self.server_id);
        for relay in &self.relay_urls {
            addr = addr.with_relay_url(relay.clone());
        }
        addr
    }
}

#[derive(Default)]
struct Fields {
    version: Option<String>,
    id: Option<String>,
    relays: Vec<String>,
    name: Option<String>,
    token: Option<String>,
}

impl Fields {
    /// 不认识的参数忽略，同一个版本里可以往码里加可选的字段。
    fn push(&mut self, key: &str, value: String) {
        match key {
            "v" => self.version = Some(value),
            "id" => self.id = Some(value),
            "relay" => self.relays.push(value),
            "name" => self.name = Some(value),
            "token" => self.token = Some(value),
            _ => {}
        }
    }

    fn build(self) -> Result<PeerPairingCode, InvalidPeerCode> {
        let server_id = self
            .id
            .as_deref()
            .ok_or_else(|| malformed("出码方 ID", "缺少 id"))
            .and_then(|id| {
                EndpointId::from_str(id).map_err(|e| malformed("出码方 ID", &e.to_string()))
            })?;
        let relay_urls = self
            .relays
            .iter()
            .map(|relay| {
                RelayUrl::from_str(relay).map_err(|e| malformed("中继地址", &e.to_string()))
            })
            .collect::<Result<Vec<_>, _>>()?;
        if relay_urls.is_empty() {
            return Err(malformed("出码方地址", "没有中继地址"));
        }
        let server_name = self
            .name
            .as_deref()
            .map(str::trim)
            .filter(|name| !name.is_empty() && name.len() <= 253)
            .ok_or_else(|| malformed("服务器名", "缺少 name 或为空"))?
            .to_owned();
        let token = self
            .token
            .as_deref()
            .and_then(PeerPairToken::from_hex)
            .ok_or_else(|| malformed("令牌", "应为 32 位十六进制小写"))?;
        Ok(PeerPairingCode {
            server_id,
            relay_urls,
            server_name,
            token,
        })
    }
}

fn malformed(field: &'static str, reason: &str) -> InvalidPeerCode {
    InvalidPeerCode::Malformed {
        field,
        reason: reason.to_owned(),
    }
}

/// 出码方的请求令牌：[`TOKEN_LEN`] 字节，十六进制小写。`Debug` 不打印原文。
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PeerPairToken(String);

impl PeerPairToken {
    /// 用给定的随机字节组成令牌。
    pub fn from_bytes(bytes: [u8; TOKEN_LEN]) -> Self {
        Self(bytes.iter().map(|b| format!("{b:02x}")).collect())
    }

    pub fn expose(&self) -> &str {
        &self.0
    }

    /// 从二维码或请求里的十六进制文字读回令牌，格式不对时为空。
    pub fn from_hex(text: &str) -> Option<Self> {
        let valid = text.len() == TOKEN_LEN * 2
            && text
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
        valid.then(|| Self(text.to_owned()))
    }
}

impl fmt::Debug for PeerPairToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<redacted>")
    }
}

/// 配对双方在配对时交换的身份。发出去的一端是自己的，收到的一端是对方的。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PeerIdentity {
    /// 自己的 Matrix 服务器名。
    pub server_name: String,
    /// 自己主人的 Matrix 用户 ID，如 `@alice:home.example`。
    pub owner_user_id: String,
    /// 自己主人的显示名。
    pub owner_display_name: String,
    /// 能被对方回连的地址：中继在前，直连跟上（取法照设备配对码）。
    #[serde(default)]
    pub relays: Vec<String>,
    #[serde(default)]
    pub addrs: Vec<SocketAddr>,
}

impl PeerIdentity {
    /// 回连这一端用的地址。
    pub fn endpoint_addr(&self, id: EndpointId) -> EndpointAddr {
        let mut addr = EndpointAddr::new(id);
        for relay in &self.relays {
            if let Ok(relay) = RelayUrl::from_str(relay) {
                addr = addr.with_relay_url(relay);
            }
        }
        for ip in &self.addrs {
            addr = addr.with_ip_addr(*ip);
        }
        addr
    }
}

/// 一端经 [`PEER_PAIR_ALPN`] 发给另一端的消息。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PeerMessage {
    /// 扫码方出示令牌和自己的身份，请出码方记下这条请求。
    Scan {
        token: PeerPairToken,
        /// 扫码方的身份：服务器名、主人和回连地址。
        #[serde(flatten)]
        identity: PeerIdentity,
    },
    /// 扫码方来问自己那条请求的结果。谁问由 QUIC 握手认证的 endpoint ID 认，不用再报。
    Outcome,
    /// 出码方把主人的决定告诉扫码方。
    Decision {
        /// 主人点的是同意（`true`）还是拒绝（`false`）。
        approved: bool,
        /// 出码方的身份：服务器名、主人和回连地址。同意后两台服务器各记一条配对。
        #[serde(flatten)]
        identity: PeerIdentity,
    },
    /// 一方删了和另一方的配对（ADR-0050）：尽力而为的通知，对方离线时靠联邦握手被拒兜底。
    Removed { server_name: String },
}

/// 对 [`PeerMessage`] 的应答。`Error` 的原因 [`PeerRejectReason`] 对上设备配对的 [`crate::pair::RejectReason`]
/// 的用法：App 和核心按它提示主人该做什么。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "result", rename_all = "snake_case")]
pub enum PeerReply {
    /// 令牌是对的，请求记下了，等主人同意；附出码方的身份。
    AwaitingApproval {
        #[serde(flatten)]
        identity: PeerIdentity,
    },
    /// 问结果：请求还在等主人。
    Pending,
    /// 问结果：主人同意了，附出码方的身份。两边互相发了请求时，扫码那一步也直接回这个。
    Approved {
        #[serde(flatten)]
        identity: PeerIdentity,
    },
    /// 问结果：主人拒绝了。
    Declined,
    /// 问结果：这台服务器没有这个 endpoint ID 发来的请求（重置过码，或者已经处理掉了）。
    UnknownRequest,
    /// 决定已送达，或者删除通知已收下。
    Ok,
    Error {
        reason: PeerRejectReason,
        /// 给人看的说明。
        message: String,
    },
}

/// 出码方拒绝一次扫码的原因。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PeerRejectReason {
    /// 令牌不对：对方拿的是重置前的码，要一张新的。
    CodeRevoked,
    /// 主人拒绝过这台服务器，还没撤销。
    Rejected,
    /// 待处理的请求满了，或者这一小时收得太多。
    TooManyRequests,
    /// 这台服务器的名字已经被另一个 endpoint 占用（已经配对过）。
    AlreadyPaired,
    /// 扫了自己的服务器。
    SelfPairing,
    /// 请求不合规（格式不对、太长）。
    BadRequest,
    /// 出码方自己出错了，稍后再试。
    ServerError,
    /// 比这个核心新的服务端给出的原因。
    #[serde(other)]
    Other,
}

#[cfg(test)]
mod tests {
    use iroh::SecretKey;

    use super::*;

    fn identity() -> PeerIdentity {
        PeerIdentity {
            server_name: "abc.wonton.internal".to_owned(),
            owner_user_id: "@alice:abc.wonton.internal".to_owned(),
            owner_display_name: "Alice".to_owned(),
            relays: vec!["https://relay.example.com./".to_owned()],
            addrs: vec!["192.168.1.20:18482".parse().unwrap()],
        }
    }

    fn code() -> PeerPairingCode {
        PeerPairingCode {
            server_id: SecretKey::generate().public(),
            relay_urls: vec![RelayUrl::from_str("https://relay.example.com./").unwrap()],
            server_name: "abc.wonton.internal".to_owned(),
            token: PeerPairToken::from_bytes([0xab; TOKEN_LEN]),
        }
    }

    /// 契约：码经自己的 URI 往返，首尾空白不算数，版本是 1，不带直连地址。
    #[test]
    fn a_code_survives_its_own_uri() {
        let code = code();
        let uri = code.to_uri();
        assert!(uri.starts_with("wonton://peer?v=1&"), "{uri}");
        assert!(!uri.contains("addr="), "{uri}");
        assert_eq!(PeerPairingCode::parse(&uri).unwrap(), code);
        assert_eq!(PeerPairingCode::parse(&format!(" {uri}\n")).unwrap(), code);
    }

    /// 契约：别的二维码、旧版本和缺字段的码分得清。
    #[test]
    fn other_codes_and_broken_fields_are_told_apart() {
        for text in ["https://example.com/", "hello", "wonton://pair?v=0"] {
            assert_eq!(
                PeerPairingCode::parse(text),
                Err(InvalidPeerCode::NotPeerCode),
                "{text}"
            );
        }
        let older = code().to_uri().replace("v=1", "v=0");
        assert_eq!(
            PeerPairingCode::parse(&older),
            Err(InvalidPeerCode::UnsupportedVersion("0".to_owned()))
        );
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
            without("relay"),
            without("token"),
            without("name"),
            uri.replace("token=abab", "token=ABAB"),
        ] {
            assert!(
                matches!(
                    PeerPairingCode::parse(&broken),
                    Err(InvalidPeerCode::Malformed { .. })
                ),
                "{broken}"
            );
        }
    }

    /// 两端各自编译，消息的 JSON 形状就是契约。
    #[test]
    fn messages_have_the_documented_json_shape() {
        let scan = PeerMessage::Scan {
            token: PeerPairToken::from_bytes([1; TOKEN_LEN]),
            identity: identity(),
        };
        assert_eq!(
            serde_json::to_value(&scan).unwrap(),
            serde_json::json!({
                "type": "scan",
                "token": "01010101010101010101010101010101",
                "server_name": "abc.wonton.internal",
                "owner_user_id": "@alice:abc.wonton.internal",
                "owner_display_name": "Alice",
                "relays": ["https://relay.example.com./"],
                "addrs": ["192.168.1.20:18482"],
            })
        );
        assert_eq!(
            serde_json::to_value(PeerMessage::Outcome).unwrap(),
            serde_json::json!({"type": "outcome"})
        );
        let decision = PeerMessage::Decision {
            approved: true,
            identity: identity(),
        };
        assert_eq!(serde_json::to_value(&decision).unwrap()["type"], "decision");
        assert_eq!(
            serde_json::to_value(&PeerMessage::Removed {
                server_name: "abc.wonton.internal".to_owned(),
            })
            .unwrap(),
            serde_json::json!({"type": "removed", "server_name": "abc.wonton.internal"})
        );
        let reply: PeerReply = serde_json::from_value(serde_json::json!({
            "result": "awaiting_approval",
            "server_name": "def.wonton.internal",
            "owner_user_id": "@alex:def.wonton.internal",
            "owner_display_name": "Alex",
            "relays": [],
            "addrs": [],
        }))
        .unwrap();
        assert_eq!(
            reply,
            PeerReply::AwaitingApproval {
                identity: PeerIdentity {
                    server_name: "def.wonton.internal".to_owned(),
                    owner_user_id: "@alex:def.wonton.internal".to_owned(),
                    owner_display_name: "Alex".to_owned(),
                    relays: Vec::new(),
                    addrs: Vec::new(),
                }
            }
        );
        for (reply, expected) in [
            (PeerReply::Pending, serde_json::json!({"result": "pending"})),
            (
                PeerReply::Declined,
                serde_json::json!({"result": "declined"}),
            ),
            (
                PeerReply::UnknownRequest,
                serde_json::json!({"result": "unknown_request"}),
            ),
            (PeerReply::Ok, serde_json::json!({"result": "ok"})),
        ] {
            assert_eq!(serde_json::to_value(&reply).unwrap(), expected);
        }
        let approved: PeerReply = serde_json::from_value(serde_json::json!({
            "result": "approved",
            "server_name": "def.wonton.internal",
            "owner_user_id": "@alex:def.wonton.internal",
            "owner_display_name": "Alex",
            "relays": [],
            "addrs": [],
        }))
        .unwrap();
        assert!(matches!(approved, PeerReply::Approved { .. }));
        let error: PeerReply = serde_json::from_value(serde_json::json!({
            "result": "error", "reason": "code_revoked", "message": "码作废了",
        }))
        .unwrap();
        assert!(matches!(
            error,
            PeerReply::Error {
                reason: PeerRejectReason::CodeRevoked,
                ..
            }
        ));
        let newer: PeerReply = serde_json::from_value(serde_json::json!({
            "result": "error", "reason": "something_new", "message": "…",
        }))
        .unwrap();
        assert!(matches!(
            newer,
            PeerReply::Error {
                reason: PeerRejectReason::Other,
                ..
            }
        ));
    }

    /// 令牌不进日志。
    #[test]
    fn debug_hides_the_token() {
        let code = code();
        let debug = format!("{code:?}");
        assert!(!debug.contains(code.token.expose()), "{debug}");
        let message = PeerMessage::Scan {
            token: code.token.clone(),
            identity: identity(),
        };
        assert!(!format!("{message:?}").contains(code.token.expose()));
    }

    /// 回连地址能从身份里拼出来。
    #[test]
    fn identity_builds_an_endpoint_addr() {
        let id = SecretKey::generate().public();
        let addr = identity().endpoint_addr(id);
        assert_eq!(addr.id, id);
        assert!(
            addr.ip_addrs()
                .any(|a| a == &"192.168.1.20:18482".parse().unwrap())
        );
    }
}
