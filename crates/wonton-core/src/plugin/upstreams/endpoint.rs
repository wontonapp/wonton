//! 核心访问插件服务的地址：普通网络里是 `host:port`，宿主网络里是插件数据目录里的 unix 套接字。

use std::path::{Path, PathBuf};

use super::super::proxy_auth::McpHeaders;

/// 核心访问插件服务的地址：普通网络里是 `host:port`，宿主网络里是插件数据目录里的 unix 套接字。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Endpoint {
    Tcp(String),
    Unix(PathBuf),
}

impl Endpoint {
    pub fn unix(path: impl Into<PathBuf>) -> Self {
        Self::Unix(path.into())
    }

    /// 给 HTTP 客户端用的 URL。unix 套接字的主机名不参与解析，用 `localhost` 占位。
    pub fn url(&self, path: &str) -> String {
        match self {
            Self::Tcp(authority) => format!("http://{authority}{path}"),
            Self::Unix(_) => format!("http://localhost{path}"),
        }
    }

    pub fn socket(&self) -> Option<&Path> {
        match self {
            Self::Tcp(_) => None,
            Self::Unix(path) => Some(path),
        }
    }
}

/// 总线连一个插件的 MCP 服务要的东西：地址、解析好的请求头，宿主网络里还有套接字。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpTarget {
    pub url: String,
    pub socket: Option<PathBuf>,
    pub headers: McpHeaders,
}
