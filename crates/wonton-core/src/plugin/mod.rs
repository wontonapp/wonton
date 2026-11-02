//! 插件管理：解析 `x-wonton` 插件包、渲染部署计划、经 Docker API 安装和卸载。

pub mod bot_services;
pub mod bots;
mod by_source;
pub mod catalog;
pub mod check;
pub mod compose;
pub mod configs;
pub mod docker;
pub mod docker_exec;
pub mod gallery;
pub mod github;
pub mod github_source;
pub mod gpu;
pub mod health;
pub mod hostdir;
pub mod image_worker;
pub mod images;
mod install_error;
pub mod interpolate;
pub mod lan;
pub mod languages;
pub mod lifecycle;
pub mod manager;
pub mod manifest;
pub mod models;
pub mod mounts;
pub mod names;
pub mod package;
pub mod package_files;
pub mod preinstall;
pub mod provides;
pub mod proxy_auth;
pub mod render;
pub mod secrets;
pub mod sources;
pub mod store;
pub mod store_info;
pub mod system;
mod upstreams;
pub mod views;

#[cfg(test)]
pub(crate) mod fake_docker;
#[cfg(test)]
mod tests;

pub use manager::{Health, InstallError, McpServer, PluginManager, PluginStatus, PluginView};
pub use manifest::{Manifest, Permission, PluginId};
pub use package::PluginPackage;
