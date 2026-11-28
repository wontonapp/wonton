//! 聊天列表最上面的安装占位行（ADR-0067）：有 Bot 的插件在装、在升级、装失败时合成一行，私聊出现后撤掉。
//!
//! 数据都来自插件页的 ViewModel：已安装列表的状态和安装进度、安装任务的结果，加上任务记下的预览（来源和是不是 Bot
//! 插件）。房间列表进到这个模块里判断私聊有没有出现，壳不用去重。

mod rows;
mod state;

pub(in crate::chat) use state::Installs;

use std::time::Duration;

/// 装好以后还等不到 Bot 的私聊，过这么久就不占这一行了：插件页照样看得到它。
const SETTLE: Duration = Duration::from_secs(120);

#[cfg(test)]
mod tests;
