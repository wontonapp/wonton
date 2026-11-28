use serde::{Deserialize, Serialize};

use crate::{CredentialEntry, EntryKind, NewPasswordRequest, PutIngressRequest};

/// 第三方客户端和公网访问（ADR-0021、ADR-0089、ADR-0090）的操作；“HTTPS 和域名”页（ADR-0089）也在里面。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientsAction {
    /// 取客户端入口列表（`GET` [`clients_path`](crate::clients_path)），结果在
    /// [`ClientsViewModel::plugins`](crate::ClientsViewModel::plugins)。
    Refresh,
    /// 取后缀、服务商和证书状态；进页面时走一次。
    RefreshDomain,
    /// 离开页面：丢掉没保存的表单，回到服务器上现在存的值。保存中被离页时，这次保存照常提交。
    CancelDomain,
    /// 开或关一个入口（`PUT` [`clients_entry_path`](crate::clients_entry_path)）；`kind` 是哪个入口。
    /// [`EntryKind::Other`](crate::EntryKind::Other) 拼不出路径段：照常显示这一行，但把开关置灰、不发请求。
    SetEntry {
        id: String,
        kind: EntryKind,
        enabled: bool,
    },
    /// 保存后缀、服务商和凭据（`PUT` [`clients_domain_path`](crate::clients_domain_path)）。
    /// 保存。后缀在服务器名已经定了时只校验不改；凭据为空表示沿用已存的。
    SaveDomain {
        suffix: Option<String>,
        /// lego 的服务商代码；`None` 是“其他”。
        provider: Option<String>,
        /// 预设表单的凭据，按变量名；“其他”时为空。
        credentials: Vec<CredentialEntry>,
        /// “其他”的凭据原文，一行一个 `键=值`；预设时为空串。
        other_credentials: String,
        accept_terms: bool,
    },
    /// 取服务商预设（`GET` [`dns_providers_path`](crate::dns_providers_path)），结果在
    /// [`ClientsSettingsViewModel::providers`](crate::ClientsSettingsViewModel::providers)。
    DnsProviders,
    /// 换公网入口（`PUT` [`clients_ingress_path`]）；旧连接器先撤掉。
    SaveIngress { request: PutIngressRequest },
    /// 公网访问总开关（`PUT` [`clients_public_access_path`](crate::clients_public_access_path)）。
    SetPublicAccess { enabled: bool },
    /// 取一个应用的访客链接（`GET` [`clients_guests_path`]）。
    RefreshGuests { id: String },
    /// 新建一条访客链接（`POST` [`clients_guests_path`](crate::clients_guests_path)）。
    CreateGuest {
        id: String,
        label: String,
        ttl_seconds: u32,
        path: Option<String>,
    },
    /// 取一条访客链接的完整地址（`GET` [`clients_guest_path`]），每次取记审计；
    /// 列表里的 `url` 为空，只有这一条带。结果并进 [`ClientsGuests`]。
    GetGuest { id: String, guest: String },
    /// 收回一条访客链接（`DELETE` [`clients_guest_path`]）；下一个请求就失效。
    RevokeGuest { id: String, guest: String },
    /// 取入口、证书、闸门和自检（`GET` [`clients_status_path`](crate::clients_status_path)）。
    RefreshStatus,
    /// 新建一个专用密码（`POST` [`clients_passwords_path`]）；值先放进
    /// [`ClientsViewModel::password`](crate::ClientsViewModel::password)，刷新后才有这一条。`per_client: false` 的插件再建回 409。
    CreatePassword {
        id: String,
        request: NewPasswordRequest,
    },
    /// 取一个密码的值（`GET` [`clients_password_path`]），每次取值记审计；值放进
    /// [`ClientsViewModel::password`]。
    GetPassword { id: String, password: String },
    /// 吊销一个密码（`DELETE` [`clients_password_path`]）；`per_client: false` 时是换值。
    RevokePassword { id: String, password: String },
    /// 提前解除一个来源的封禁（`DELETE` [`clients_ban_path`]）；`source` 的写法同 [`Ban`]。
    Unban { source: String },
    /// 选中服务商；`None` 是“其他”。只改动本机表单，不写服务器。
    SelectDomainProvider { code: Option<String> },
    /// 续期失败或过期后重试一次，沿用现在的后缀、服务商和凭据。
    RetryCertificate,
    /// 收起上一次的错误。
    DismissDomainProblem,
}
