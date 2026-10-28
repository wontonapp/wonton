//! 插件管理：解析 `x-wonton` 插件包、渲染部署计划、经 Docker API 安装和卸载。

pub mod compose;
pub mod docker;
pub mod health;
pub mod hostdir;
pub mod interpolate;
pub mod lifecycle;
pub mod manager;
pub mod manifest;
pub mod package;
pub mod render;
pub mod store;

#[cfg(test)]
mod fake_docker;
#[cfg(test)]
mod tests;

pub use manager::{InstallError, PluginManager, PluginStatus, PluginView};
pub use manifest::{Manifest, Permission, PluginId};
pub use package::PluginPackage;
