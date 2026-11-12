//! 有远端联系人的 Bot 只聊天（ADR-0050“远端联系人默认只聊天”）：只要这个 Bot 有 Active 的远端联系人，
//! 列工具和调用都不给插件的工具，核心工具只给 [`ALLOWED`] 里的几个。
//!
//! 按 Bot 整体挡，不按轮次：总线的 [`Caller`] 只知道是哪个 Bot，不知道这一轮是谁的话（#353）。远端联系人删掉、
//! 或不再是 Active 时下一次列工具、调用就恢复。不另发工具列表变化的通知：名单一变核心就重建这个 Bot
//! （`Effects::allowed_users_changed`），Agent 重启后重新列工具。

use std::sync::Arc;

use rmcp::model::Tool;
use wonton_protocol::BotRef;

use super::{Bus, Caller, ToolFailure, authoring_tools, core_tools, naming, profile_tools};
use crate::contacts::Contacts;

/// 有远端联系人的 Bot 还能用的核心工具。只放只读、没有副作用、回的内容是公开的或 Bot 自己的；每一项的理由和
/// 不给的那些见 ADR-0050 的“远端联系人只聊天”。
const ALLOWED: &[&str] = &[
    core_tools::HELP,
    core_tools::SEARCH,
    authoring_tools::GUIDE,
    profile_tools::GET,
];

/// 失败类别：Agent 和 App 从 `app.wonton/failure` 读，知道是规则挡的、重试没用。
const KIND: &str = "remote_contact";

impl Bus {
    /// 接上联系人表。没接 Matrix 的核心没有联系人表，也就不会有远端联系人。
    pub fn attach_contacts(&self, contacts: Arc<Contacts>) {
        if self.contacts.set(contacts).is_err() {
            tracing::error!("联系人表接了两次，用先接上的");
        }
    }

    /// 这个调用方是有 Active 远端联系人的 Bot。不是 Bot 的插件（不进聊天）没有联系人。
    pub(super) fn remote_contact_blocked(&self, caller: &Caller) -> bool {
        let (Some(bot), Some(contacts)) = (&caller.bot, self.contacts.get()) else {
            return false;
        };
        contacts.has_active_remote(&BotRef {
            plugin_id: caller.id.to_string(),
            bot: bot.bot.clone(),
        })
    }

    /// 调用前的门：挡住时返回拒绝的原因。`owner` 是工具名里的插件段，`tool` 是去掉前缀的工具名。
    pub(super) fn remote_contact_gate(
        &self,
        caller: &Caller,
        owner: &str,
        tool: &str,
    ) -> Option<ToolFailure> {
        let allowed = owner == naming::CORE && ALLOWED.contains(&tool);
        (!allowed && self.remote_contact_blocked(caller)).then(|| {
            ToolFailure::new(
                KIND,
                format!(
                    "这个 Bot 有别的服务器上的联系人，按规则只聊天：不能用插件的工具和会改东西、读家里数据的核心工具\
                     （{}）。重试也一样，要用的话请归属人先删掉远端联系人。",
                    naming::join(owner, tool)
                ),
            )
        })
    }
}

/// 有远端联系人的 Bot 列到的工具：[`ALLOWED`] 里的核心工具。
pub(super) fn tools() -> Vec<Tool> {
    core_tools::definitions()
        .into_iter()
        .filter(|tool| {
            naming::split(&tool.name)
                .is_some_and(|(owner, name)| owner == naming::CORE && ALLOWED.contains(&name))
        })
        .collect()
}
