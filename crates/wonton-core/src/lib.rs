//! 服务端核心：插件管理、网关与配对、推送代理、MCP 总线、密钥与存储授权。
//!
//! 设计见 `docs/ARCHITECTURE.md` 与 `docs/adr/0006-plugin-system.md`。

// 真实 Docker 测试的共用辅助（#[cfg(test)] 的 test_docker）对单元测试和 tests/ 里的集成测试都按
// `wonton_core::` 引用，本 crate 内部给自己起这个别名，两种编译单元共用同一份文件。
#[cfg(test)]
extern crate self as wonton_core;

#[cfg(test)]
pub mod test_docker;
#[cfg(test)]
mod test_docker_check;

mod bot_activity;
pub mod bot_media;

pub mod config;
pub mod device;
pub mod files;
pub mod gateway;
pub mod help;
pub mod http;
pub mod install;
pub(crate) mod l10n;
pub mod matrix;
pub mod mcp;
pub mod members;
pub mod oidc;
pub mod owner;
pub mod pairing;
pub mod peers;
pub mod plugin;
pub mod push;
pub mod secret;
pub mod tunnel;
pub mod upgrade;
