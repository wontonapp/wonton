//! 安装、升级、卸载：一个插件同时只有一个任务，进展放在 `PluginsViewModel.tasks` 里。家里的服务器单独执行这些操作，请求
//! 断开了也照样做完（ADR-0017），所以连接断了不算失败，要从已安装列表里等结果。

mod confirm;
mod finish;
mod rows;
mod run;

pub(super) use super::api;
pub(super) use super::identity;
pub(super) use super::together;
pub(super) use super::{POLL_INTERVAL, PluginsLink};
pub(super) use confirm::problem;
pub(super) use finish::{Outcome, server_changed};
