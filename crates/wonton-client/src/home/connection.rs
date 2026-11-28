//! 要告诉主人的连接状态（ADR-0032）：把隧道状态、系统网络和中继的情况算成一句话。只有判断，没有 IO，时间由调用方给。

mod notice;
mod observe;
mod recovery;
#[cfg(test)]
mod tests;
mod timing;
mod tracker;

#[allow(unused_imports)] // 只有测试用。
pub(super) use observe::Recovery;
pub(super) use timing::LONG_OUTAGE;
pub(super) use tracker::Tracker;
