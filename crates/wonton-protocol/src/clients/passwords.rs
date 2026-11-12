//! 专用密码（ADR-0021“专用密码”）：列表、新建、取值、吊销，以及吊销设备时的保留选择。

use serde::{Deserialize, Serialize};

use crate::SecretString;

/// 一个专用密码的概况，值另外取（取值记审计）。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct PasswordSummary {
    pub id: String,
    /// 新建时主人起的名字，客户端就用它区分。
    pub label: String,
    /// 建它的设备名；不知道时为空。
    pub created_by_device: Option<String>,
    /// 看过它的设备名，按最近一次取值排。
    pub last_viewed_by: Vec<String>,
    /// 吊销或换值还没完成；完成前照旧能用，不能当已吊销。
    pub pending: Option<PendingOp>,
}

/// 还没做完的吊销或换值；失败和超时都算结果未知，核心会重试。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum PendingOp {
    Revoke,
    /// `per_client: false` 的插件吊销时是换值。
    Rotate,
    /// 比这个 App 新的核心给的操作。
    #[serde(other)]
    Other,
}

/// [`clients_passwords_path`](crate::clients_passwords_path) 的请求体。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct NewPasswordRequest {
    /// 客户端起个名字（如“家里的 iPad”），方便以后单独吊销。
    pub label: String,
}

/// [`clients_password_path`](crate::clients_password_path) 的响应。值是 128 位随机数的小写 base32。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct PasswordValue {
    pub username: String,
    /// 复制时标成敏感内容。
    pub password: SecretString,
}

/// 吊销一台设备（`DELETE /_wonton/api/devices/{endpoint_id}`）的请求体；不带请求体时字段取默认值。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct RevokeDeviceRequest {
    /// 这台设备看过或建过的共享密码（`per_client: false`）不换值、只吊销它们；默认换值。
    /// 没被这台设备碰过的不动。响应里列出待完成的吊销和登录没结束的应用。
    #[serde(default)]
    pub keep_shared_passwords: bool,
}
