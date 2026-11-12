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

pub mod agent_chain;
mod bot_activity;
pub mod bot_media;

pub mod config;
pub mod contacts;
pub mod device;
pub mod entry_certificate;
pub mod entry_compose;
pub mod entry_dns;
pub mod entry_runtime;
pub mod entry_settings;
pub mod files;
pub mod friends;
pub mod gateway;
pub mod help;
pub mod home_service;
pub mod http;
pub mod install;
pub(crate) mod l10n;
pub(crate) mod link_preview;
pub mod mailbox;
pub mod matrix;
pub mod mcp;
pub mod members;
pub mod net;
pub mod oidc;
pub mod owner;
pub mod pairing;
pub mod peers;
pub mod plugin;
pub mod push;
pub mod secret;
pub mod share_identity;
pub mod shares;
pub mod tunnel;
pub mod upgrade;
