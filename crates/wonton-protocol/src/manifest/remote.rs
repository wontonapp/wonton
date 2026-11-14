use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// `x-wonton.remote_mcp`。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RemoteMcpSpec {
    /// 远端 MCP 服务器的 Streamable HTTP 地址。
    pub url: String,
    #[serde(default)]
    pub auth: RemoteAuth,
    /// `auth: header` 时核心发往提供方的每个请求都带上的头：名字 → 值模板，只能引用 `secrets` 里的 `input` 项。
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub headers: BTreeMap<String, String>,
    /// 只接这一份工具（上游的原名），同 `mcp.tools`。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_idle_seconds: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_total_seconds: Option<u64>,
}

/// 远端服务器怎么认我们。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RemoteAuth {
    #[default]
    None,
    /// 请求头带主人填的密钥（ADR-0064）。
    Header,
    /// OAuth：App 拿授权码交给连接器，令牌只在连接器里（ADR-0066 第 3a 节）。
    Oauth,
}

impl RemoteMcpSpec {
    /// 主机名（带端口）：确认页和详情页写“数据发到 <主机>”。
    pub fn host(&self) -> &str {
        self.url
            .strip_prefix("https://")
            .unwrap_or(&self.url)
            .split(['/', '?', '#'])
            .next()
            .unwrap_or_default()
    }

    /// 给 App 的鉴权方式。
    pub fn protocol_auth(&self) -> crate::RemoteAuth {
        match self.auth {
            RemoteAuth::None => crate::RemoteAuth::None,
            RemoteAuth::Header => crate::RemoteAuth::Header,
            RemoteAuth::Oauth => crate::RemoteAuth::Oauth,
        }
    }
}
