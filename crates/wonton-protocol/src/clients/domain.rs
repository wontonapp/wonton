//! 自己的域名和证书（ADR-0089）：后缀能不能改、服务商的凭据有没有、证书、CAA 和局域网记录现在对不对。

use serde::{Deserialize, Serialize};

use crate::SecretString;

/// `GET` [`clients_domain_path`](crate::clients_domain_path) 的响应。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct DomainSettings {
    /// 入口用的后缀；还没配时为空。
    pub suffix: Option<String>,
    /// 装机时用了自己的域名（它同时是 Matrix 服务器名）时为假：后缀改不了，只能换服务商和凭据。
    pub suffix_editable: bool,
    /// lego 的服务商代码；没配时为空。
    pub provider: Option<String>,
    /// 服务商的凭据已经存过；只写不读。
    pub credentials_set: bool,
    /// 主人勾过 Let's Encrypt 的订户协议。
    pub terms_accepted: bool,
    pub certificate: CertificateState,
    pub caa: CaaState,
    pub lan_record: LanRecordState,
}

/// `PUT` [`clients_domain_path`](crate::clients_domain_path) 的请求体。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct PutDomainRequest {
    /// 新的后缀；服务器名已经定了、只换凭据时为空。
    pub suffix: Option<String>,
    /// lego 的服务商代码（如 `desec`）。
    pub provider: String,
    /// 服务商要的变量，值是密钥；变量名照服务商预设，核心只校验字符集。
    pub credentials: Vec<CredentialEntry>,
    /// 勾了订户协议才下单。
    pub accept_terms: bool,
}

/// 服务商要的一个变量。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct CredentialEntry {
    /// 变量名（如 `DESEC_TOKEN`）。
    pub name: String,
    pub value: SecretString,
}

/// 证书现在到哪一步；过期后如实说连接会失败，不降级到自签名或 `http`。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum CertificateState {
    None,
    Issuing,
    Valid {
        /// 到期时刻（Unix 毫秒）。
        not_after_ms: u64,
    },
    /// 续期失败，还在用旧证书；App 写到什么时候和 lego 的错误。
    Renewing {
        not_after_ms: u64,
        error: String,
    },
    Expired {
        not_after_ms: u64,
        error: String,
    },
    /// 比这个 App 新的核心给的状态。
    #[serde(other)]
    Other,
}

/// 后缀上的 CAA 记录对不对（ADR-0021“CT 日志监控”）；只是推荐，不对也能签。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum CaaState {
    /// 已经限定到这个家的 ACME 账号。
    Bound,
    /// 没有 CAA 记录，给出建议的那一条。
    Missing,
    /// 有 CAA，但没限定账号或者写的是别家。
    Mismatch,
    /// 查不出来（服务商没答、超时）。
    Unknown,
    /// 比这个 App 新的核心给的状态。
    #[serde(other)]
    Other,
}

/// `*.lan.<后缀>` 的 A 记录对不对；打开任何局域网入口时查一次。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum LanRecordState {
    Ok,
    Missing,
    /// 记录在，但指向的不是这台主机的局域网地址。
    Wrong {
        /// 现在解析出来的地址。
        found: String,
    },
    Unknown,
    /// 比这个 App 新的核心给的状态。
    #[serde(other)]
    Other,
}

/// 一个服务商预设（数据，不写代码，ADR-0089 第 2 节）。预设来自固定的 lego 版本的 provider 清单。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct DnsProviderPreset {
    /// lego 的 provider 代码。
    pub code: String,
    /// 显示名。
    pub name: String,
    /// 要填的变量名，按清单的顺序；值都是密钥。
    pub credential_vars: Vec<String>,
    /// 去哪注册；不需要时为空。
    pub signup_url: Option<String>,
    /// 默认推荐的那一家，App 排在最前。
    pub recommended: bool,
}
