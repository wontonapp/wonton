//! 插件管理：解析 `x-wonton` 插件包、渲染部署计划、经 Docker API 安装和卸载。

mod bot_avatars;
pub mod bot_profile;
pub mod bot_services;
pub mod bots;
mod by_source;
pub mod check;
pub mod compose;
pub mod configs;
pub(super) mod configs_store;
pub mod core_address;
#[cfg(test)]
mod core_address_tests;
pub mod docker;
pub mod docker_exec;
mod docker_limits;
pub mod egress;
mod entry;
pub mod gallery;
pub mod github;
pub mod github_source;
pub mod gpu;
pub mod health;
pub mod health_probe;
pub mod host_check;
pub mod host_facts;
pub mod host_relay;
pub mod hostdir;
pub mod image_worker;
pub mod images;
pub mod index_location;
mod install_entry;
mod install_error;
pub mod interpolate;
pub mod lan;
pub mod languages;
mod leftovers;
pub mod lifecycle;
pub mod limits;
mod limits_update;
mod listed;
pub mod manager;
mod manager_options;
pub mod manifest;
mod manifest_agent;
mod manifest_mcp;
mod manifest_oidc;
mod manifest_tabs;
mod mcp_server;
pub(crate) mod model_providers;
pub mod models;
pub mod mounts;
pub mod names;
mod network_migration;
mod network_mode;
pub mod network_pool;
pub mod network_pool_command;
#[cfg(test)]
mod network_pool_probe_tests;
#[cfg(test)]
mod network_pool_tests;
pub(crate) mod oom;
pub mod package;
pub mod package_archive;
pub mod package_files;
pub mod preinstall;
pub mod privileges;
pub mod provides;
pub mod proxy_auth;
mod reconcile;
pub mod registry_mirror;
pub mod render;
pub mod requirements;
pub mod save_grants;
pub mod secrets;
pub mod source_address;
pub mod sources;
pub mod staged;
mod start;
pub mod store;
pub mod store_info;
pub mod storefront;
pub mod system;
#[cfg(test)]
mod tests_support;
pub mod tool_policy;
mod upgrade;
pub(crate) mod upstreams;
mod vars;
pub mod views;
mod views_permissions;
mod views_source;
mod watch;
pub mod written_by;

#[cfg(test)]
pub(crate) mod fake_docker;
#[cfg(test)]
mod tests;

pub use manager::{Health, InstallError, PluginManager, PluginStatus, PluginView};
pub use manifest::{Manifest, Permission, PluginId};
pub(crate) use manifest_tabs::validate_path;
pub use mcp_server::{McpServer, ToolLimits};
pub use package::PluginPackage;
pub use proxy_auth::McpHeaders;
pub use upgrade::UpgradeTo;
