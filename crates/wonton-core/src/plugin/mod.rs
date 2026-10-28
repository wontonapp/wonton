//! 插件管理：解析 `x-wonton` 插件包、渲染部署计划、经 Docker API 安装和卸载。

pub mod bot_services;
pub mod compose;
pub mod configs;
pub mod docker;
pub mod health;
pub mod hostdir;
pub mod interpolate;
pub mod lifecycle;
pub mod manager;
pub mod manifest;
pub mod mounts;
pub mod package;
pub mod package_files;
pub mod render;
pub mod secrets;
pub mod store;

#[cfg(test)]
mod fake_docker;
#[cfg(test)]
mod tests;

pub use manager::{InstallError, PluginManager, PluginStatus, PluginView};
pub use manifest::{Manifest, Permission, PluginId};
pub use package::PluginPackage;
