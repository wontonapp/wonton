//! 插件管理：解析 `x-wonton` 插件包、渲染部署计划、经 Docker API 安装和卸载。

pub mod bot_services;
pub mod bots;
pub mod catalog;
pub mod compose;
pub mod configs;
pub mod docker;
pub mod docker_exec;
pub mod health;
pub mod hostdir;
mod install_error;
pub mod interpolate;
pub mod lifecycle;
pub mod manager;
pub mod manifest;
pub mod models;
pub mod mounts;
pub mod package;
pub mod package_files;
pub mod provides;
pub mod proxy_auth;
pub mod render;
pub mod secrets;
pub mod sources;
pub mod store;
pub mod system;
mod upstreams;

#[cfg(test)]
pub(crate) mod fake_docker;
#[cfg(test)]
mod tests;

pub use manager::{Health, InstallError, McpServer, PluginManager, PluginStatus, PluginView};
pub use manifest::{Manifest, Permission, PluginId};
pub use package::PluginPackage;
