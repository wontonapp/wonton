//! 第三方客户端和公网访问（ADR-0021、ADR-0089、ADR-0090）：入口开关、专用密码、域名、公网入口、访客链接和入口状态。
//!
//! 接口都在 `/_wonton/clients` 下，经应用内隧道、只给已配对的设备，而且都要求的是主人的设备（ADR-0021 的“接口”）。
//! 每个请求带 [`CLIENTS_HEADER`]（值 `1`）：隧道身份随 WebView 的请求自动带上，跨站带自定义请求头却要先过预检，
//! 插件网页因此碰不到这些接口。错误体是 [`ClientsErrorBody`]，类别也写进 [`CLIENTS_ERROR_HEADER`]。
//!
//! | 方法 | 路径 | 类型 |
//! | --- | --- | --- |
//! | GET | [`clients_path`] | `GET` 的响应是 `Vec<`[`ClientsPlugin`]`>` |
//! | PUT | [`clients_entry_path`] | 请求体 [`SetEntryRequest`] |
//! | POST | [`clients_passwords_path`] | 请求体 [`NewPasswordRequest`] |
//! | GET、DELETE | [`clients_password_path`] | `GET` 的响应是 [`PasswordValue`] |
//! | GET、PUT | [`clients_domain_path`] | `GET` 的响应是 [`DomainSettings`]，`PUT` 的请求体是 [`PutDomainRequest`] |
//! | GET | [`dns_providers_path`] | `GET` 的响应是 `Vec<`[`DnsProviderPreset`]`>` |
//! | GET、PUT | [`clients_ingress_path`] | `GET` 的响应是 [`IngressSettings`]，`PUT` 的请求体是 [`PutIngressRequest`] |
//! | PUT | [`clients_public_access_path`] | 请求体 [`PublicAccess`]，公网访问总开关 |
//! | GET、POST | [`clients_guests_path`] | `POST` 的请求体是 [`NewGuestLinkRequest`] |
//! | GET、DELETE | [`clients_guest_path`] | `GET` 的响应是 [`GuestLink`] |
//! | GET | [`clients_status_path`] | `GET` 的响应是 [`ClientsStatus`] |
//! | DELETE | [`clients_ban_path`] | 解除一个来源的封禁 |
//!
//! 核心可能比 App 新：核心多给的字段被忽略；枚举遇到不认识的取值时落到 `Other`，不让整份状态解析失败。

mod domain;
mod entries;
mod failure;
mod guests;
mod ingress;
mod passwords;
mod status;
#[cfg(test)]
mod tests;

pub use domain::{
    CaaState, CertificateState, CredentialEntry, DnsProviderPreset, DomainSettings, LanRecordState,
    PutDomainRequest,
};
pub use entries::{
    ClientAuthKind, ClientsPlugin, EntryBlock, EntryKind, EntrySwitch, PublicRuleMode, PublicRules,
    SetEntryRequest,
};
pub use failure::{CLIENTS_ERROR_HEADER, ClientsErrorBody, ClientsFailure, ClientsProblem};
pub use guests::{
    GUEST_LINKS_MAX, GUEST_TTL_DEFAULT, GUEST_TTL_MAX, GuestLink, NewGuestLinkRequest,
};
pub use ingress::{
    GateState, IngressKind, IngressSettings, PublicAccess, PublicSwitchState, PutIngressRequest,
};
pub use passwords::{
    NewPasswordRequest, PasswordSummary, PasswordValue, PendingOp, RevokeDeviceRequest,
};
pub use status::{
    Ban, ClientsStatus, PendingChange, PendingChangeKind, ProbeResult, PublicItem, PublicReach,
};

/// 所有第三方客户端接口都要带的请求头，值是 `1`；不带的请求（跨站）过不了。
pub const CLIENTS_HEADER: &str = "X-Wonton-Clients";

/// `GET`：每个声明了 `clients` 或 `webui.guests` 的插件一行，响应是 `Vec<`[`ClientsPlugin`]`>`。只给主人。
pub fn clients_path() -> String {
    "/_wonton/clients".into()
}

/// `PUT`：开或关一个入口，请求体是 [`SetEntryRequest`]。`entry` 取 [`EntryKind::as_path_segment`]：
/// 局域网是 `lan`，公网是 `public`，`clients.tcp` 是清单里的名字。`credentials` 还没成功发过密码时回 409
/// [`ClientsFailure::Conflict`](crate::ClientsFailure)。
pub fn clients_entry_path(id: &str, entry: &str) -> String {
    format!("{}/{id}/entries/{entry}", clients_path())
}

/// `POST`：新建一个专用密码，请求体是 [`NewPasswordRequest`]；`per_client: false` 的插件只有一个，再建回 409。
pub fn clients_passwords_path(id: &str) -> String {
    format!("{}/{id}/passwords", clients_path())
}

/// `GET`：取值，响应是 [`PasswordValue`]，每次取值记审计（具体到设备）。`DELETE`：吊销；`per_client: false` 时是换值。
/// 吊销没完成时回 202，状态里的 [`PendingChange`](crate::PendingChange) 如实显示为待完成。
pub fn clients_password_path(id: &str, password: &str) -> String {
    format!("{}/{id}/passwords/{password}", clients_path())
}

/// `GET`：后缀、DNS 服务商和证书状态，响应是 [`DomainSettings`]。`PUT`：写入，请求体是 [`PutDomainRequest`]。
pub fn clients_domain_path() -> String {
    format!("{}/domain", clients_path())
}

/// `GET`：可选的服务商预设，响应是 `Vec<`[`DnsProviderPreset`]`>`。
pub fn dns_providers_path() -> String {
    format!("{}/providers", clients_domain_path())
}

/// `GET`：入口的种类和它的设置，响应是 [`IngressSettings`]。`PUT`：换入口，请求体是 [`PutIngressRequest`]。
pub fn clients_ingress_path() -> String {
    format!("{}/ingress", clients_path())
}

/// `PUT`：公网访问总开关，请求体是 [`PublicAccess`]。关掉时三道闸门逐一执行，
/// 没关干净的写进 [`ClientsStatus::gates`](crate::ClientsStatus)。
pub fn clients_public_access_path() -> String {
    format!("{}/public-access", clients_path())
}

/// `GET`：这个插件的访客链接，响应是 `Vec<`[`GuestLink`]`>`。`POST`：新建，请求体是 [`NewGuestLinkRequest`]。
pub fn clients_guests_path(id: &str) -> String {
    format!("{}/{id}/guests", clients_path())
}

/// `GET`：取地址，响应是 [`GuestLink`]，每次取记审计。`DELETE`：收回，下一个请求就失效。
pub fn clients_guest_path(id: &str, guest: &str) -> String {
    format!("{}/{id}/guests/{guest}", clients_path())
}

/// `GET`：入口、证书、自检、封禁和待完成的事情，响应是 [`ClientsStatus`]。
pub fn clients_status_path() -> String {
    format!("{}/status", clients_path())
}

/// `DELETE`：解除一个来源的封禁。`source` 是 IPv4 地址或 IPv6 的 /64（`2001:db8::/64` 这种写法）。
pub fn clients_ban_path(source: &str) -> String {
    format!("{}/bans/{source}", clients_path())
}
