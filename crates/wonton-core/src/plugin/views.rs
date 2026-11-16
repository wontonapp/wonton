//! 管理接口给 App 的视图（ADR-0017 的“接口：App 和核心”）：已安装的插件、详情页、安装预览和图标。文字按请求的
//! 语言挑好，App 原样显示。

mod detail;
mod install;
mod list;
mod text;

pub(crate) use super::views_agent_logins::agent_login_options;
pub(crate) use super::views_permissions::permission_line;
pub use super::views_permissions::permission_text;

pub(in crate::plugin) use detail::requirement_checks;
pub(crate) use text::cross_origin_isolation_notice;
pub(in crate::plugin) use text::{author, kind, line};
