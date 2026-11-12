use std::sync::Arc;

use wonton_protocol::BotContactsViewModel;

use super::ClientCore;
use crate::BotContactsSubscription;

#[uniffi::export]
impl ClientCore {
    /// 当前的 Bot 联系人页（ADR-0045、ADR-0050），用于首帧渲染。
    pub fn bot_contacts(&self) -> BotContactsViewModel {
        self.views().bot_contacts.borrow().clone()
    }

    /// 订阅 Bot 联系人页，语义同 [`subscribe_session_list`](ClientCore::subscribe_session_list)。
    pub fn subscribe_bot_contacts(&self) -> Arc<BotContactsSubscription> {
        Arc::new(BotContactsSubscription::new(
            self.views().bot_contacts.subscribe(),
        ))
    }
}
