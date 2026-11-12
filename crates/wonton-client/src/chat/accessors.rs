//! 聊天的视图读写和订阅入口；壳只在必要时才拿发送端。

use super::*;

impl ChatLink {
    pub(crate) fn view(&self) -> ChatViewModel {
        self.view.borrow().clone()
    }

    /// 换服务器时新作用域接着用这个发送端，壳的订阅不断（ADR-0042）。
    pub(crate) fn sender(&self) -> watch::Sender<ChatViewModel> {
        self.view.clone()
    }

    pub(crate) fn subscribe(&self) -> watch::Receiver<ChatViewModel> {
        self.view.subscribe()
    }

    pub(crate) fn sharing_sessions(&self) -> watch::Receiver<Option<Arc<MatrixSession>>> {
        self.bots.session.subscribe()
    }

    pub(crate) fn subscribe_sent(&self) -> watch::Receiver<Vec<String>> {
        self.sent.subscribe()
    }

    pub(crate) fn set_primary_bot(&self, bot: Option<String>) {
        let changed = {
            let mut current = lock(&self.primary_bot);
            if *current == bot {
                false
            } else {
                *current = bot.clone();
                true
            }
        };
        if changed {
            let rows = self.room_order.display(bot.as_deref());
            self.view.send_modify(|view| view.rooms = rows);
            self.installs.refresh();
        }
    }
}

impl ChatLink {
    /// 「等你确认」列表的发送端，同上。
    pub(crate) fn confirmations_sender(&self) -> watch::Sender<ConfirmationsViewModel> {
        self.confirmations.clone()
    }
}
