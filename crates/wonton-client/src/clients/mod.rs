//! App 内“客户端”这一块的操作入口。
//!
//! 页面挂在设置里，只有一个：[`ClientsDomainLink`]（HTTPS 和域名，ADR-0089）。

mod api;
mod domain;

pub(crate) use domain::ClientsDomainLink;
