use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// 清单的 `clients`：HTTP 入口最多一个，TCP 入口若干。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClientsSpec {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub http: Option<ClientsHttp>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tcp: Option<Vec<ClientsTcp>>,
}

/// `clients.http`：经 TLS 入口转给插件网页服务的 HTTP 入口。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClientsHttp {
    pub service: String,
    pub port: u16,
    /// 只认一个地址的应用：三条路上的 `Host` 都改写成 `WONTON_EXTERNAL_URL` 的主机名（ADR-0089 第 5 节）。
    #[serde(default, skip_serializing_if = "is_false")]
    pub single_host: bool,
    pub auth: ClientsAuth,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub public: Option<ClientsPublic>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_failures: Option<Vec<AuthFailure>>,
}

/// `clients.http.auth`：gateway、credentials、app 恰好一个。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClientsAuth {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gateway: Option<GatewayAuth>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credentials: Option<CredentialsAuth>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app: Option<AppAuth>,
}

/// 网关验证：认 `Basic` 和 `Bearer`，通过后删掉 `Authorization`、按模板加请求头。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GatewayAuth {
    /// 加在转发请求上的请求头：名字 → 值模板，模板同 `webui.proxy.headers`。
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
}

/// 应用验证、核心发密码：在插件的容器里跑声明的命令，看退出码。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CredentialsAuth {
    /// 覆盖写：把标准输入给的值设成这个人的密码，不结束已有的登录。必须能重复执行。
    pub issue: Vec<String>,
    /// 标准输入给值，退出码 0 表示有效。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verify: Option<Vec<String>>,
    /// 吊销。`per_client: true` 时必写。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revoke: Option<Vec<String>>,
    /// 换值。`per_client: false` 时必写。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rotate: Option<Vec<String>>,
    /// 吊销和换值结不结束应用里已有的登录。必写，不设默认值。
    pub ends_sessions: bool,
    /// 跑命令的服务，默认 `clients.http.service`。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service: Option<String>,
    /// 显示给主人填的用户名，默认 `{user}`。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    /// 每个客户端一个密码；`false` 时整个插件一个。
    #[serde(default = "default_true")]
    pub per_client: bool,
    /// 值由应用生成（写到标准输出的第一行），不由核心生成。
    #[serde(default)]
    pub generates: bool,
}

/// 应用自己的账号：核心不发密码，App 只给地址。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppAuth {
    /// 首次注册完成没有：退出码 0 是有了，1 是还没有。首次注册没完成时入口打不开（ADR-0022）。
    pub setup_done: Vec<String>,
}

/// `clients.http.public`：公网入口的路径限制，`allow` 和 `deny` 二选一。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClientsPublic {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deny: Option<Vec<String>>,
}

/// `clients.http.auth_failures` 的一项：哪些响应算验证失败。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthFailure {
    pub path: String,
    pub status: Vec<u16>,
}

/// `clients.tcp` 的一项：非 TLS 协议或自己带 TLS 的协议（ADR-0021“普通 TCP 转发”）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClientsTcp {
    pub service: String,
    pub port: u16,
    /// 入口的名字，开关和接口路径里用它。
    pub name: String,
}

fn default_true() -> bool {
    true
}

fn is_false(value: &bool) -> bool {
    !*value
}
