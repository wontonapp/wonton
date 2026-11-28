//! 断开后的宽限和诊断时限。

use std::time::Duration;

#[allow(unused_imports)] // 只为下面文档里的链接。
use super::Tracker;

/// 断开不到这么久不提示：挡掉抖动和冷启动时的闪烁。
pub(super) const GRACE: Duration = Duration::from_secs(2);
/// 断开这么久才说“好像离线了”。和 iroh 连接级的空闲时限对齐：30 秒还连不上，多半不是路径在切换。
pub(in crate::home) const LONG_OUTAGE: Duration = Duration::from_secs(30);
/// 中继掉线这么久才算连不上。iroh 1.4.1 实测：家里的服务器不通时，模拟器上每隔几十秒掉线几秒就回来，不等一等，
/// “好像离线了”和“连不上”会来回跳。
pub(super) const RELAY_GRACE: Duration = Duration::from_secs(10);
/// 手动重试至少显示一秒，最长只显示一个诊断窗口；没结果也要重新给出下一步。
pub(super) const RETRY_SPIN: Duration = Duration::from_secs(1);
/// 没连上时多久重新算一次：断开的时长和中继的情况在变，它们不发事件。
pub(super) const TICK: Duration = Duration::from_secs(1);
/// 连上后“已恢复”的提示显示这么久，期间 [`HomeViewModel::just_recovered`](wonton_protocol::HomeViewModel::just_recovered)
/// 为真；由 [`Tracker::next_tick`] 保证到点就有一次重算把它翻回去。
pub(super) const RECOVERY_FEEDBACK: Duration = Duration::from_secs(4);
