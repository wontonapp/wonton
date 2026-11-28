//! 插件页面的行为测试：只经由导出给原生壳的接口驱动，隧道的对面是本机假扮的核心管理接口，只走回环地址。

mod agent_dispatch;
mod agent_stop;
mod ask_every_update;
mod audience;
mod bots;
mod cached_views;
mod capability_install;
mod capability_uninstall;
mod chat_rows;
mod connection;
mod core_upgrade;
mod detail;
mod dropped_results;
mod egress;
mod failure_categories;
mod install;
mod list;
mod now;
mod offline_tasks;
mod onboarding;
mod owner_secrets;
mod profile;
mod registry_mirror;
pub mod rig;
mod server_switch;
mod settle;
mod share_recipients;
mod stale_list;
mod store_load;
mod store_sources;
mod takeover;
mod tool_policy;

use axum::http::Method;
use wonton_client::{Action, PluginFailure, PluginOperation, PluginState};

use rig::{Rig, WAIT, core_error, installed, preview};
