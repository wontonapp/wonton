//! 访客链接（ADR-0090 第 5 节）：给“发个链接就能进”的应用用的临时匿名入口。

use serde::{Deserialize, Serialize};

/// 默认有效期 4 小时。
pub const GUEST_TTL_DEFAULT: u32 = 4 * 60 * 60;
/// 最长有效期 7 天，超过回 [`ClientsFailure::InvalidInput`](crate::ClientsFailure)。
pub const GUEST_TTL_MAX: u32 = 7 * 24 * 60 * 60;
/// 一个应用最多 20 条有效链接。
pub const GUEST_LINKS_MAX: usize = 20;

/// [`clients_guests_path`](crate::clients_guests_path) 的 `POST` 请求体。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct NewGuestLinkRequest {
    /// 起个名字，方便以后单独收回。
    pub label: String,
    /// 有效期，秒；取 [`GUEST_TTL_DEFAULT`] 时照默认，超过 [`GUEST_TTL_MAX`] 回 400。
    pub ttl_seconds: u32,
    /// 打开后跳到的路径（如房间）；不带就停在应用的首页。
    pub path: Option<String>,
}

/// 一条访客链接。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct GuestLink {
    pub id: String,
    pub label: String,
    /// 到期时刻（Unix 毫秒）；总开关关着时也照走，不顺延。
    pub expires_at_ms: u64,
    pub path: Option<String>,
    /// 最近一次被用的时刻（Unix 毫秒）；没用过时为空。
    pub last_used_at_ms: Option<u64>,
    /// 完整地址 `<对外地址>/_wonton/guest/<令牌>`；取这一条时才有，列表里为空。
    pub url: Option<String>,
}
