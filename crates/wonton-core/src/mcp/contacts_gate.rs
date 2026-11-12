//! 有远端联系人的 Bot 只聊天（ADR-0050“远端联系人默认只聊天”）：这个 Bot 进行中的桥轮次里有别的服务器上的人
//! 说话，列工具和调用都不给插件的工具，核心工具只给 [`ALLOWED`] 里的几个；归属人自己那一轮照常。
//!
//! 一句话同时引出的几个轮次取最低档（ADR-0083 第 5 节）：只要有一个远端轮还活着，同一个 Bot 别的轮次也挡。
//! 进行中的轮次只认调用链账本里桥报的（ADR-0061 第 10 节），不信 Agent 自报的来源：那能伪造成归属人私聊。
//! 没有进行中的轮次时回落到“有 Active 的远端联系人”（#365）：定时任务、报不了轮次的 Agent 走这条。
//!
//! 判定在调用时做，`tools/list` 按同一规则过滤；不发工具列表变化的通知：轮次秒级切换，通知只有抖动，
//! 旧列表里的工具被调时得到 `remote_contact` 的拒绝，Agent 能解释。

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

    /// 这个调用方此刻只许聊天：它进行中的桥轮次里有别的服务器上的人，或者它没有进行中的轮次、表里却有
    /// Active 的远端联系人（回落）。不是 Bot 的插件（不进聊天）没有联系人，也没有轮次。
    pub(super) fn remote_contact_blocked(&self, caller: &Caller) -> bool {
        let (Some(bot), Some(contacts)) = (&caller.bot, self.contacts.get()) else {
            return false;
        };
        let bot = BotRef {
            plugin_id: caller.id.to_string(),
            bot: bot.bot.clone(),
        };
        // 判不准的一律往低处算：认不出服务器名的说话人也算远端（证明不了是本机的人）。
        if let Some(turns) = self
            .agent_chains()
            .map(|chains| chains.ledger().live_bridge_turns(&bot))
            .filter(|turns| !turns.is_empty())
        {
            return turns.iter().any(|turn| contacts.is_remote(&turn.sender));
        }
        contacts.has_active_remote(&bot)
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
                    "这个 Bot 此刻在跟别的服务器上的人说话，按规则只聊天：不能用插件的工具和会改东西、\
                     读家里数据的核心工具（{}）。归属人自己那一轮照常。",
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
