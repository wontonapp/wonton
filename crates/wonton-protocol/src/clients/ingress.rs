//! 公网入口的种类和总开关（ADR-0090）：同一时间只用一种入口，令牌只写不读；总开关关掉时三道闸门各自报状态。

use serde::{Deserialize, Serialize};

use crate::SecretString;

/// 公网连接怎么进到家里；不靠 Cloudflare 那边的配置决定放行哪个标签。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(rename_all = "snake_case")]
pub enum IngressKind {
    /// 路由器把一个公网端口转到主机，家里的 TCP 转发容器接。
    RouterForward,
    /// 主人自己的服务器上跑原版 frps，家里用 frpc 连过去。
    OwnFrps,
    /// Cloudflare Tunnel，TLS 在 Cloudflare 边缘终止；`clients.tcp` 不可用。
    CloudflareTunnel,
    /// 比这个 App 新的核心给的入口。
    #[serde(other)]
    Other,
}

/// `GET` [`clients_ingress_path`](crate::clients_ingress_path) 的响应：主人选的入口和它不保密的设置。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct IngressSettings {
    /// 选的入口；还没选时为空，公网入口都打不开。
    pub kind: Option<IngressKind>,
    /// 路由器要转到主机的端口；别的入口为空。
    pub port: Option<u16>,
    /// 自己服务器：地址、端口、frps 证书里的名字；别的入口为空。
    pub server_addr: Option<String>,
    pub server_port: Option<u16>,
    pub server_name: Option<String>,
    /// 自己服务器的 CA 证书（PEM），用来校验 frps；别的入口为空。
    pub ca_pem: Option<String>,
    /// 令牌存过没有（frps 的登录令牌、Cloudflare 的隧道令牌）；只写不读。
    pub token_set: bool,
}

/// `PUT` [`clients_ingress_path`](crate::clients_ingress_path) 的请求体；换入口时旧的连接器先撤掉再建新的。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PutIngressRequest {
    RouterForward {
        /// 路由器上转过来的端口，用可读标签就不必是 443。
        port: u16,
    },
    OwnFrps {
        server_addr: String,
        server_port: u16,
        /// frps 证书里的名字，frpc 拿它校验对方。
        server_name: String,
        /// frps 的登录令牌，是密钥。
        token: SecretString,
        /// 签发 frps 证书的 CA（PEM）。
        ca_pem: String,
    },
    CloudflareTunnel {
        /// 远程管理的隧道令牌，只能跑这一条隧道。
        token: SecretString,
    },
}

/// 公网访问总开关；默认关，只有主人在 App 里能开（ADR-0090 第 2 节）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct PublicAccess {
    pub enabled: bool,
}

/// 总开关关掉时要分别做完的三道闸门；关不干净的一道如实报，不合并成“已关”。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct PublicSwitchState {
    /// 删掉连接器容器。
    pub connector: GateState,
    /// 清空入口容器公网前端的 `crt-list`。
    pub crt_list: GateState,
    /// 在 8484 上拒绝 `X-Wonton-Entry: public`，断开已有的公网连接。
    pub core: GateState,
}

/// 一道闸门现在的状态。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum GateState {
    Done,
    Failed {
        /// 核心按 App 的语言写的原因。
        detail: String,
    },
    /// 还没做或者查不出来：不能当已关。
    Unknown,
    /// 比这个 App 新的核心给的状态。
    #[serde(other)]
    Other,
}
