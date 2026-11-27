use serde::{Deserialize, Serialize};

/// 网关拒绝请求时在此头写类别；Matrix 规则拒绝为 403，分享身份失败为 401，其他分享拒绝为 403。
pub const GATEWAY_REFUSAL_HEADER: &str = "x-wonton-error";

/// 跳数到顶（[`GatewayRefusal::HopLimit`]）时 403 响应体的 `errcode`。Bot 用的 Matrix SDK 读不到响应头，
/// `M_FORBIDDEN` 的其余字段也会丢掉，只有自定义的 `errcode` 原样留在错误里，Bot 一侧靠它认出类别。
pub const HOP_LIMIT_ERRCODE: &str = "APP.WONTON.HOP_LIMIT";

/// 网关在 403 响应体的 `errcode` 里写 [`GatewayRefusal`] 的前缀（`APP.WONTON.CONTACT_REQUIRED`）。
/// 响应头 [`GATEWAY_REFUSAL_HEADER`] 照旧写，给读得到头的一方用；客户端 SDK 读不到头，靠 `errcode`。
pub const GATEWAY_REFUSAL_ERRCODE_PREFIX: &str = "APP.WONTON.";

/// 网关拒绝 Matrix 或分享请求的类别，写在 [`GATEWAY_REFUSAL_HEADER`] 里。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GatewayRefusal {
    /// 邀请的对象不是这个 Bot 听的人，也不是它的联系人。
    ContactRequired,
    /// 建公开房间，或把房间改成公开。
    PublicRoom,
    /// 人上次在这个房间说话之后，不同 Bot 之间来回说得太多了。
    HopLimit,
    /// OpenID token 失效、发现失败或 userinfo 的服务器名不符（401）。
    ShareIdentity,
    /// 没有给这个好友开放该插件（403）。
    ShareRequired,
    ShareRevoked,
    ShareExpired,
    #[serde(other)]
    Other,
}

impl GatewayRefusal {
    /// 响应头里的写法。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ContactRequired => "contact_required",
            Self::PublicRoom => "public_room",
            Self::HopLimit => "hop_limit",
            Self::ShareIdentity => "share_identity",
            Self::ShareRequired => "share_required",
            Self::ShareRevoked => "share_revoked",
            Self::ShareExpired => "share_expired",
            Self::Other => "other",
        }
    }

    /// 网关在 403 响应体 `errcode` 里写的完整错误码。Matrix SDK 读不到响应头，客户端靠它认类别。
    pub fn errcode(self) -> &'static str {
        match self {
            Self::ContactRequired => "APP.WONTON.CONTACT_REQUIRED",
            Self::PublicRoom => "APP.WONTON.PUBLIC_ROOM",
            Self::HopLimit => HOP_LIMIT_ERRCODE,
            Self::ShareIdentity => "APP.WONTON.SHARE_IDENTITY",
            Self::ShareRequired => "APP.WONTON.SHARE_REQUIRED",
            Self::ShareRevoked => "APP.WONTON.SHARE_REVOKED",
            Self::ShareExpired => "APP.WONTON.SHARE_EXPIRED",
            Self::Other => "APP.WONTON.OTHER",
        }
    }

    /// 从 `errcode` 认回类别；不是网关拒绝的码时为空（按普通 403 处理）。
    pub fn from_errcode(errcode: &str) -> Option<Self> {
        Some(
            match errcode.strip_prefix(GATEWAY_REFUSAL_ERRCODE_PREFIX)? {
                "CONTACT_REQUIRED" => Self::ContactRequired,
                "PUBLIC_ROOM" => Self::PublicRoom,
                "HOP_LIMIT" => Self::HopLimit,
                "SHARE_IDENTITY" => Self::ShareIdentity,
                "SHARE_REQUIRED" => Self::ShareRequired,
                "SHARE_REVOKED" => Self::ShareRevoked,
                "SHARE_EXPIRED" => Self::ShareExpired,
                "OTHER" => Self::Other,
                _ => return None,
            },
        )
    }
}
