//! 连接的观察结果。

use std::time::Duration;

/// 一次断开恢复了。
#[derive(Debug, PartialEq, Eq)]
pub(in crate::home) struct Recovery {
    pub(in crate::home) outage: Duration,
    pub(in crate::home) trigger: &'static str,
}
