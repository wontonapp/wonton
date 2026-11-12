//! 核心的一键升级（ADR-0025）。
//!
//! 核心不能替换自己，所以真正动手的是一次性的升级器容器（[`updater`]）；核心这边（[`service`]）查新版本、发起、看住
//! 升级器，升级没结束时冻结插件。两边只经状态文件（[`state`]）说话：核心断开期间状态还在。

pub mod backup;
pub mod check;
pub mod compat;
pub mod components;
pub mod compose_images;
pub mod docker_host;
pub mod envfile;
#[cfg(test)]
pub(crate) mod fake_host;
pub mod host;
pub mod manual;
mod observe;
pub mod service;
#[cfg(test)]
mod service_tests;
pub mod state;
pub mod updater;
#[cfg(test)]
mod updater_tests;

/// 升级器容器的名字。
pub const UPDATER_CONTAINER: &str = "wonton-updater";
/// 系统服务的 compose 项目和核心的服务名：跨版本的接口（ADR-0025 的“版本之间的约定”）。
pub const PROJECT: &str = "wonton";
pub const CORE_SERVICE: &str = "core";
/// 镜像的版本号。
pub const LABEL_VERSION: &str = "org.opencontainers.image.version";
/// 镜像支持的其他组件，逗号分隔。
pub const LABEL_COMPONENTS: &str = "wonton.core.components";
/// 镜像声明的不可逆迁移：`tuwunel@0.5.0,hydra@0.6.0`。
pub const LABEL_MIGRATIONS: &str = "wonton.core.migrations";
/// 核心管理的其他组件的容器都打这个标签，值是 `core`。
pub const LABEL_MANAGED_BY: &str = "wonton.managed-by";
/// 经 Docker API 建的组件的名字；compose 项目的名字就是组件名。
pub const LABEL_COMPONENT: &str = "wonton.core.component";
