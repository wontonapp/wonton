//! “HTTPS 和域名”设置页的 ViewModel（ADR-0089 第 2–4 节）：页面结构和值在核心，壳只渲染。
//!
//! 凭据的值只上行：表单里输入的值由壳当场带走，ViewModel 只给变量名和“存过没有”。

use serde::{Deserialize, Serialize};

use super::{CaaState, CertificateState, ClientsProblem, LanRecordState};

/// 页面当前拿不拿得到设置。加载中和失败都不画表单。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum ClientsDomainState {
    Loading,
    Ready,
    Failed {
        problem: ClientsProblem,
    },
    /// 比这个 App 新的核心给的状态。
    #[serde(other)]
    Other,
}

/// “HTTPS 和域名”设置页。后缀、服务商、证书、CAA 和局域网记录都在这里。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ClientsDomainViewModel {
    pub state: ClientsDomainState,
    /// 入口现在用的后缀；还没配时为空。
    pub suffix: Option<String>,
    /// 假：后缀就是 Matrix 服务器名，只能换服务商和凭据。
    pub suffix_editable: bool,
    /// 服务商选项：预设按顺序在前（deSEC 第一），最后一项是“其他”，`code` 为空。
    pub providers: Vec<ClientsDomainProvider>,
    /// 现在选中的服务商代码；“其他”或还没配时为空。
    pub selected_provider: Option<String>,
    /// 选中的是“其他”，壳显示 lego 代码和 `键=值` 凭据两栏。
    pub other_provider: bool,
    /// 选中的预设要哪些凭据变量，按顺序。
    pub credential_vars: Vec<String>,
    /// 服务器上已经存过凭据；只写不读。
    pub credentials_set: bool,
    /// 主人勾过 Let's Encrypt 的订户协议。
    pub terms_accepted: bool,
    pub certificate: ClientsDomainCertificate,
    /// CAA 现在对不对；建议加时 `record` 是整条记录。
    pub caa: CaaState,
    /// 建议加的那条 CAA 记录（整条，核心拼好）；已经限定到本机账号时不给。
    pub caa_record: Option<String>,
    pub lan_record: LanRecordState,
    /// `*.lan.<后缀>` 里“要加的那一条”的名字部分；没有后缀时为空。
    pub lan_record_name: Option<String>,
    /// 这条 A 记录该指向的地址，取自局域网检测选中的那一个；还没选出来时为空，壳显示占位。
    pub lan_target: Option<String>,
    /// 选中的是 deSEC：壳在“详情”里放注册和限定令牌的步骤。
    pub desec: bool,
    /// 正在保存。
    pub saving: bool,
    /// 上一次操作被拒的原因；成功刷新后清掉。
    pub problem: Option<ClientsProblem>,
}

impl Default for ClientsDomainViewModel {
    fn default() -> Self {
        Self {
            state: ClientsDomainState::Loading,
            suffix: None,
            suffix_editable: false,
            providers: Vec::new(),
            selected_provider: None,
            other_provider: false,
            credential_vars: Vec::new(),
            credentials_set: false,
            terms_accepted: false,
            certificate: ClientsDomainCertificate::default(),
            caa: CaaState::Unknown,
            caa_record: None,
            lan_record: LanRecordState::Unknown,
            lan_record_name: None,
            lan_target: None,
            desec: false,
            saving: false,
            problem: None,
        }
    }
}

/// 一个服务商选项。预设是数据（ADR-0089 第 2 节），“其他”的 `code` 为空。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ClientsDomainProvider {
    /// lego 的 provider 代码；空是“其他”。
    pub code: Option<String>,
    /// 显示名；预设的品牌名不翻译，“其他”由壳写。
    pub name: String,
    /// 默认推荐的一家，排在最前。
    pub recommended: bool,
    /// 去哪注册；不需要时为空。
    pub signup_url: Option<String>,
    /// 这家要填的凭据变量，按顺序。
    pub credential_vars: Vec<String>,
}

/// 证书状态和失败时的“重试 + 详情”。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct ClientsDomainCertificate {
    pub state: CertificateState,
    /// 失败过（续期失败或已过期）：壳给人话和重试按钮。
    pub retryable: bool,
    /// lego 的原始错误，放进默认收起的“详情”。
    pub details: Option<String>,
}
