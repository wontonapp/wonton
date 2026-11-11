//! 总线按联系人挡插件工具（ADR-0050“远端联系人默认不能用插件”）。字段在 [`Bus`] 上，接线和判断在这里。

use std::sync::Arc;

use wonton_protocol::BotRef;

use super::{Bus, Caller};

impl Bus {
    /// 挂上联系人表。没挂时（有些测试）按没有远端联系人处理。
    pub fn attach_contacts(&self, contacts: Arc<crate::contacts::Contacts>) {
        let _ = self.contacts.set(contacts);
    }

    /// 这个 Bot 有 Active 的远端联系人吗：有就不给插件工具。认不出一轮是谁的话——总线没有消息
    /// 来源——按 ADR-0045 同款的先例取 Bot 粒度，方向一致、更保守。
    pub(super) fn remote_contact_blocked(&self, caller: &Caller) -> bool {
        let Some(bot) = &caller.bot else {
            return false;
        };
        self.contacts.get().is_some_and(|contacts| {
            contacts.bot_with_active_remote(&BotRef {
                plugin_id: caller.id.to_string(),
                bot: bot.bot.clone(),
            })
        })
    }

    /// 挡掉的回复：列工具时已经藏起来，这里兜住直接调的。
    pub(super) fn remote_contact_denial(tool: &str) -> super::ToolFailure {
        super::ToolFailure::new(
            "remote_contact",
            format!("这个 Bot 有另一台服务器上的联系人，插件工具用不了（{tool}）"),
        )
    }
}
