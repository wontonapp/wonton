//! 核心和插件之间的入口：核心在插件网络里的别名（插件经它们访问核心转发的服务），以及核心访问插件的地址
//! （网页、模型网关、MCP 服务）。普通网络里连 [`super::names::service_host`] 的稳定域名（上游看到的 Host 就是它），
//! 宿主网络里的服务经 [`super::host_relay`] 的 unix 套接字访问。

mod address;
mod aliases;
mod deploy;
mod endpoint;
mod probe;
mod resource;
mod routes;
mod webui;

pub(super) use aliases::{core_aliases, uses_sso};
#[allow(unused_imports)] // 只有 deploy 自己用，路径留着和别的入口一致
pub(super) use deploy::check_present;
pub use endpoint::{Endpoint, McpTarget};
pub(super) use probe::{PROBE_TIMEOUT, probe, status};
pub use resource::ResourceUpstreamError;
