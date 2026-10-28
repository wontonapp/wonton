//! 服务端核心：插件管理、网关与配对、推送代理、MCP 总线、密钥与存储授权。
//!
//! 设计见 `docs/ARCHITECTURE.md` 与 `docs/adr/0006-plugin-system.md`。M0 实现了插件管理器、
//! 网关和小程序单点登录；配对、推送代理和 MCP 总线在后续里程碑加入。

pub mod config;
pub mod gateway;
pub mod http;
pub mod oidc;
pub mod plugin;
pub mod secret;
