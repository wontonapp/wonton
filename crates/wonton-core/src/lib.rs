//! 服务端核心：插件管理、网关与配对、推送代理、MCP 总线、密钥与存储授权。
//!
//! 设计见 `docs/ARCHITECTURE.md` 与 `docs/adr/0006-plugin-system.md`。

pub mod config;
pub mod device;
pub mod files;
pub mod gateway;
pub mod help;
pub mod http;
pub mod install;
pub mod matrix;
pub mod mcp;
pub mod oidc;
pub mod owner;
pub mod pairing;
pub mod plugin;
pub mod push;
pub mod secret;
pub mod tunnel;
pub mod upgrade;
