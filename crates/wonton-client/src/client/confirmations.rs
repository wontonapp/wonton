//! 「等你确认」列表（ADR-0016）给壳的入口，单独放一个文件，`client.rs` 不超过行数上限。

use std::sync::Arc;

use wonton_protocol::ConfirmationsViewModel;

use super::*;
use crate::subscription::ConfirmationsSubscription;

#[uniffi::export]
impl ClientCore {
    /// 当前的“等你确认”列表，用于首帧渲染。
    pub fn confirmations(&self) -> ConfirmationsViewModel {
        self.views().confirmations.borrow().clone()
    }

    /// 订阅“等你确认”列表，语义同 [`super::ClientCore::subscribe_session_list`]。
    pub fn subscribe_confirmations(&self) -> Arc<ConfirmationsSubscription> {
        Arc::new(ConfirmationsSubscription::new(
            self.views().confirmations.subscribe(),
        ))
    }
}
