//! 跨服务器配对（ADR-0050）：主人出码、对方扫码、出码方同意，两台服务器的联邦就走 iroh。
//!
//! 核心的管理接口 `/_wonton/api/peers` 收发这里的类型，只收主人设备经隧道来的请求（ADR-0017 的设备门）。
//! 客户端核心原样交给壳。
//!
//! 核心可能比 App 新：核心多给的字段被忽略；枚举遇到不认识的取值时落到 `Other`，不让整份列表解析失败。

use serde::{Deserialize, Serialize};

use crate::SecretString;

/// `GET /_wonton/api/peers` 的响应。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct PeersView {
    /// 配对过的服务器，按配对时间排。
    pub peers: Vec<PeerRow>,
    /// 主人出了码、还没过期时是这张码。
    pub code: Option<PeerCode>,
}

/// 配对过的一台服务器。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct PeerRow {
    /// 对方的 Matrix 服务器名。配对时和对方的 endpoint ID 绑定，以后不变（ADR-0050）。
    pub server_name: String,
    /// 对方主人的 Matrix 用户 ID。
    pub owner_user_id: String,
    /// 对方主人的显示名。
    pub owner_display_name: String,
    pub status: PeerStatus,
    /// 最近一次连上的时刻（Unix 毫秒）；还没连上过时为空。
    pub last_connected_ms: Option<u64>,
    /// 核心按 App 的语言写的“详情”：endpoint ID 前 8 位、最后一次错误。没有可说的时为空。
    pub detail: Option<String>,
}

/// 一台对方服务器现在的连接状态，壳按类别显示文字（ADR-0043）。Tuwunel 分不清对方离线和拒绝，由核心按转发组件的状态分（ADR-0050）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum PeerStatus {
    /// 刚配对，第一次探活还没出结果：既不算连上，也不算连不上。
    Checking,
    /// 最近一次探活成功。
    Connected,
    /// 连不上对方，或探活超时。消息会在连上后补发。
    Offline,
    /// 握手后被对方的服务器名单拒绝：对方删了配对。
    RemovedByPeer,
    /// 我扫了对方的码，对方还没点同意。
    AwaitingApproval,
    /// 比这个 App 新的核心给的状态。
    #[serde(other)]
    Other,
}

/// 一张配对二维码（ADR-0050），`POST /_wonton/api/peers/code` 的响应。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct PeerCode {
    /// 二维码的内容（`wonton://peer?…`），带一次性密钥，和设备配对码一样打码；格式由核心定，壳只显示。
    pub code: SecretString,
    /// 过期时间，Unix 毫秒。
    pub expires_at_ms: u64,
}

/// `POST /_wonton/api/peers` 的请求体：提交扫来的配对码。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct PeerScan {
    /// 扫来的码（`wonton://peer?…`），带一次性密钥，和扫码配对一样打码（[`Action::PairHome`](crate::Action::PairHome)）。
    pub code: SecretString,
}

/// 错误响应里写类别的响应头，值是 [`PeerError::as_str`]。
pub const PEER_ERROR_HEADER: &str = "X-Wonton-Error";

/// `/_wonton/api/peers` 的路径。
pub const PEERS_PATH: &str = "/_wonton/api/peers";

/// 出码接口的路径。
pub fn peer_code_path() -> String {
    format!("{PEERS_PATH}/code")
}

/// 删除一台服务器的配对的路径（`DELETE`）。
pub fn peer_path(server_name: &str) -> String {
    format!("{PEERS_PATH}/{server_name}")
}

/// 配对接口失败的原因。JSON 写法就是 HTTP 错误体的 `kind` 和 [`PEER_ERROR_HEADER`] 的值；
/// 给没配对的服务器上的人发邀请时，网关把 Tuwunel 的 403 转成 [`PeerError::NotPaired`](Self::NotPaired)（ADR-0050）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum PeerError {
    /// 码过期了，要重新出码。
    CodeExpired,
    /// 码读不懂：不是配对码，或者被改过。
    CodeInvalid,
    /// 扫码后连不上出码方。
    PeerUnreachable,
    /// 出码方点了拒绝。
    PeerRejected,
    /// 两台服务器已经配对过。
    AlreadyPaired,
    /// 扫了自己服务器的码。
    SelfPairing,
    /// 这台服务器还没和对方配对（ADR-0050 的“没配对”一行）。
    NotPaired,
    /// 不是经隧道的已配对设备，或者不是主人的设备：配对接口只收主人设备（ADR-0017 的设备门）。
    Unauthenticated,
    /// 客户端核心连不上家里的服务器：隧道没连上、网络断开或请求超时。不会出现在核心的响应里。
    TransportUnavailable,
    /// 客户端核心读不懂核心的响应。不会出现在核心的响应里。
    Protocol,
    /// 比这个 App 新的核心给的类别。
    #[serde(other)]
    Other,
}

impl PeerError {
    /// 类别的写法。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::CodeExpired => "code_expired",
            Self::CodeInvalid => "code_invalid",
            Self::PeerUnreachable => "peer_unreachable",
            Self::PeerRejected => "peer_rejected",
            Self::AlreadyPaired => "already_paired",
            Self::SelfPairing => "self_pairing",
            Self::NotPaired => "not_paired",
            Self::Unauthenticated => "unauthenticated",
            Self::TransportUnavailable => "transport_unavailable",
            Self::Protocol => "protocol",
            Self::Other => "other",
        }
    }

    /// 配对接口用哪个 HTTP 状态码返回这个类别；只有客户端核心才有的类别为空。
    pub fn http_status(self) -> Option<u16> {
        match self {
            Self::CodeExpired => Some(410),
            Self::CodeInvalid | Self::SelfPairing => Some(400),
            Self::PeerUnreachable => Some(502),
            Self::PeerRejected | Self::NotPaired => Some(403),
            Self::AlreadyPaired => Some(409),
            Self::Unauthenticated => Some(401),
            Self::TransportUnavailable | Self::Protocol | Self::Other => None,
        }
    }
}

/// 配对接口的错误响应体：`{"error": {"kind": …, "message": …}}`。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PeerErrorBody {
    pub error: PeerProblem,
}

/// 错误体里的一条：类别加核心按 App 的语言写的说明（ADR-0043）。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PeerProblem {
    pub kind: PeerError,
    pub message: String,
}

/// “其他服务器”页面（ADR-0050）：配对过的服务器，以及主人的出码。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct PeersViewModel {
    pub state: PeersState,
    /// 配对过的服务器，按配对时间排。
    pub peers: Vec<PeerRow>,
    /// 主人出了码、还没过期时是这张码，壳画成二维码。
    pub code: Option<PeerCode>,
}

/// 配对页的状态。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PeersState {
    /// 正在取列表。
    Loading,
    /// 列表是最新的。
    Ready,
    /// 取列表或上一次操作失败；`message` 是核心按 App 的语言写的说明。
    Failed { error: PeerError, message: String },
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::{Action, SettingsEntry};

    fn row() -> PeerRow {
        PeerRow {
            server_name: "abc12345.wonton".into(),
            owner_user_id: "@alex:def67890.wonton".into(),
            owner_display_name: "Alex".into(),
            status: PeerStatus::Connected,
            last_connected_ms: Some(1_799_000_000_000),
            detail: Some("endpoint 9f8e7d6c，上次错误：探活超时".into()),
        }
    }

    /// 契约：列表、出码、扫码请求、设置入口和操作都能经 JSON 往返。
    #[test]
    fn peers_types_round_trip_through_json() {
        let view = PeersView {
            peers: vec![row()],
            code: Some(PeerCode {
                code: SecretString::new("wonton://peer?k=one-time-secret"),
                expires_at_ms: 1_799_000_600_000,
            }),
        };
        let json = serde_json::to_string(&view).unwrap();
        assert_eq!(serde_json::from_str::<PeersView>(&json).unwrap(), view);

        let scan = PeerScan {
            code: SecretString::new("wonton://peer?k=one-time-secret"),
        };
        let json = serde_json::to_string(&scan).unwrap();
        assert_eq!(serde_json::from_str::<PeerScan>(&json).unwrap(), scan);

        let entry = SettingsEntry::Peers { count: 2 };
        assert_eq!(
            serde_json::from_value::<SettingsEntry>(serde_json::to_value(&entry).unwrap()).unwrap(),
            entry
        );

        let model = PeersViewModel {
            state: PeersState::Ready,
            peers: vec![row()],
            code: None,
        };
        let json = serde_json::to_string(&model).unwrap();
        assert_eq!(
            serde_json::from_str::<PeersViewModel>(&json).unwrap(),
            model
        );

        let model = PeersViewModel {
            state: PeersState::Failed {
                error: PeerError::PeerUnreachable,
                message: "连不上对方的服务器".into(),
            },
            peers: Vec::new(),
            code: None,
        };
        let json = serde_json::to_string(&model).unwrap();
        assert_eq!(
            serde_json::from_str::<PeersViewModel>(&json).unwrap(),
            model
        );

        for action in [
            Action::RefreshPeers,
            Action::ShowPeerCode,
            Action::ScanPeerCode {
                code: SecretString::new("wonton://peer?k=one-time-secret"),
            },
            Action::RemovePeer {
                server_name: "abc12345.wonton".into(),
            },
        ] {
            let json = serde_json::to_string(&action).unwrap();
            assert_eq!(serde_json::from_str::<Action>(&json).unwrap(), action);
        }
    }

    /// 契约：核心比 App 新时，不认识的状态落到 `Other`，整份列表还解析得动。
    #[test]
    fn an_unknown_status_falls_back_to_other() {
        let view: PeersView = serde_json::from_value(json!({
            "peers": [
                {
                    "server_name": "abc12345.wonton",
                    "owner_user_id": "@alex:def67890.wonton",
                    "owner_display_name": "Alex",
                    "status": "hibernating",
                    "last_connected_ms": 1_799_000_000_000u64,
                    "detail": null,
                }
            ],
            "code": null,
        }))
        .unwrap();
        assert_eq!(view.peers[0].status, PeerStatus::Other);
    }

    /// 契约：核心比 App 新时，不认识的错误落到 `Other`；类别的写法就是 [`PEER_ERROR_HEADER`]
    /// 和错误体 `kind` 的值。
    #[test]
    fn unknown_errors_fall_back_to_other_and_kinds_are_wire_strings() {
        let error: PeerError = serde_json::from_value(json!("code_lost")).unwrap();
        assert_eq!(error, PeerError::Other);

        for (kind, expected) in [
            ("code_expired", PeerError::CodeExpired),
            ("code_invalid", PeerError::CodeInvalid),
            ("peer_unreachable", PeerError::PeerUnreachable),
            ("peer_rejected", PeerError::PeerRejected),
            ("already_paired", PeerError::AlreadyPaired),
            ("self_pairing", PeerError::SelfPairing),
            ("not_paired", PeerError::NotPaired),
            ("unauthenticated", PeerError::Unauthenticated),
            ("other", PeerError::Other),
        ] {
            assert_eq!(expected.as_str(), kind);
            let parsed: PeerError = serde_json::from_value(json!(kind)).unwrap();
            assert_eq!(parsed, expected);
        }

        let body: PeerErrorBody = serde_json::from_value(json!({
            "error": {"kind": "code_invalid", "message": "码读不懂"}
        }))
        .unwrap();
        assert_eq!(
            body.error,
            PeerProblem {
                kind: PeerError::CodeInvalid,
                message: "码读不懂".into(),
            }
        );
        assert_eq!(
            serde_json::to_value(&body.error).unwrap(),
            json!({"kind": "code_invalid", "message": "码读不懂"})
        );
    }
}
