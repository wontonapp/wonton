//! 确认卡片列表（ADR-0016）和收件箱（ADR-0119）给壳的入口，单独放一个文件，`client.rs` 不超过行数上限。

use std::sync::Arc;

use wonton_protocol::{ConfirmationsViewModel, InboxViewModel};

use super::*;
use crate::subscription::{ConfirmationsSubscription, InboxSubscription};

#[uniffi::export]
impl ClientCore {
    /// 当前的卡片列表，用于首帧渲染。
    pub fn confirmations(&self) -> ConfirmationsViewModel {
        self.views().confirmations.borrow().clone()
    }

    /// 订阅“等你确认”列表，语义同 [`super::ClientCore::subscribe_session_list`]。
    pub fn subscribe_confirmations(&self) -> Arc<ConfirmationsSubscription> {
        Arc::new(ConfirmationsSubscription::new(
            self.views().confirmations.subscribe(),
        ))
    }

    /// 当前的收件箱，用于首帧渲染。确认卡片是其中一类条目；铃铛的数字是 `list.counts.needing`。
    pub fn inbox(&self) -> InboxViewModel {
        self.views().inbox.borrow().clone()
    }

    /// 订阅收件箱，语义同 [`super::ClientCore::subscribe_session_list`]。
    pub fn subscribe_inbox(&self) -> Arc<InboxSubscription> {
        Arc::new(InboxSubscription::new(self.views().inbox.subscribe()))
    }
}
