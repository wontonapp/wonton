//! 「等你确认」列表的假核心（ADR-0016）：卡片塞在 [`Home`] 里，测试从这里改和数。

use std::sync::atomic::Ordering;

use serde_json::Value;

use super::Home;

impl Home {
    /// 设置假核心的“等你确认”列表：卡片 JSON 的字段同 `ConfirmationCard`。revision 加一，
    /// 挂着的取列表请求会回来。
    pub fn set_confirmations(&self, pending: Vec<Value>, finished: Vec<Value>) {
        let mut list = self.core.confirmations.lock().unwrap();
        list.pending = pending;
        list.finished = finished;
        list.revision += 1;
        self.core.confirmations_revision.send_replace(list.revision);
    }

    /// 换掉列表里的联系人请求（`PendingContactRequest` 的 JSON），revision 跟着变。
    pub fn set_contact_requests(&self, requests: Vec<Value>) {
        let mut list = self.core.confirmations.lock().unwrap();
        list.contact_requests = requests;
        list.revision += 1;
        self.core.confirmations_revision.send_replace(list.revision);
    }

    /// 接下来这么多次取列表的请求回 502，像隧道那一次没连上。
    pub fn fail_confirmations(&self, count: usize) {
        self.core
            .failing_confirmations
            .store(count, Ordering::SeqCst);
    }

    /// 取列表的请求次数，数的是这个测试会话里的全部。
    pub fn confirmations_requests(&self) -> usize {
        self.core.confirmations_requests.load(Ordering::SeqCst)
    }

    /// 单张卡片接口回 404（`true`）或照常（`false`）。
    pub fn hide_single_confirmation(&self, missing: bool) {
        self.core
            .single_confirmation_missing
            .store(missing, Ordering::SeqCst);
    }

    /// 接下来这么多次单张卡片接口回 503，像核心忙不过来。
    pub fn fail_single_confirmation(&self, count: usize) {
        self.core
            .failing_single_confirmation
            .store(count, Ordering::SeqCst);
    }

    /// 单张卡片接口的请求次数。
    pub fn single_confirmation_requests(&self) -> usize {
        self.core
            .single_confirmation_requests
            .load(Ordering::SeqCst)
    }
}

/// 模拟的核心记下的东西。
/// 假核心记着的确认卡片。测试直接塞核心的卡片 JSON，字段同 `ConfirmationCard`。
#[derive(Default, Clone)]
pub(super) struct FakeConfirmations {
    pub(super) pending: Vec<Value>,
    pub(super) finished: Vec<Value>,
    pub(super) contact_requests: Vec<Value>,
    pub(super) revision: u64,
}
impl Home {
    /// 确认卡片收到的回答：卡片 ID 和请求体。
    pub fn confirmation_answers(&self) -> Vec<(String, Value)> {
        self.core.answers.lock().unwrap().clone()
    }

    /// 放行时段收到的“停止”：卡片 ID。
    pub fn window_stops(&self) -> Vec<String> {
        self.core.window_stops.lock().unwrap().clone()
    }

    /// 下一个回答或者“停止”这样回。
    pub fn reply_to_next_answer(&self, status: u16, body: Value) {
        self.core
            .answer_replies
            .lock()
            .unwrap()
            .push_back((status, body));
    }

    pub fn hold_answers(&self, hold: bool) {
        self.core.hold_answers.send_replace(hold);
    }
}
