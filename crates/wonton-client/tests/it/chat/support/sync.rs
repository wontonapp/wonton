//! 同步请求的记录和挂起，加入事件报成受邀的改写。

use std::sync::atomic::Ordering;

use serde_json::Value;

use super::super::stale_join::StaleJoin;
use super::Home;
use super::wait::WAIT;

impl Home {
    /// arm 起之后、有任何一条同步应答带过 `stale` 房间的加入事件（连接被丢弃也算）起，每个这样的应答都被改写
    /// 成还是受邀（同步重启后 SDK 会重发房间状态，会话里多条连接都会带回它），直到
    /// [`Self::clear_join_report_as_invited`] 清除。
    pub fn report_join_as_invited(&self, stale: StaleJoin) {
        self.core.stale_join.send_replace(Some(stale));
    }

    /// 停止改写：之后的同步应答照原样转发，房间回到真实状态。
    pub fn clear_join_report_as_invited(&self) {
        self.core.stale_join.send_replace(None);
    }

    /// [`Self::report_join_as_invited`] 有没有改写到应答。
    pub fn join_was_reported_as_invited(&self) -> bool {
        self.core.join_reported_as_invited.load(Ordering::SeqCst) > 0
    }

    /// 记下的同步请求体，按客户端进入本地代理的顺序；独立隧道流的到达顺序不代表发送顺序。
    pub fn sync_requests(&self) -> Vec<Value> {
        let mut requests = self.core.sync_requests.lock().unwrap().clone();
        requests.sort_by_key(|request| request["test_sequence"].as_u64().unwrap());
        requests
    }

    pub fn hold_next_unsubscribed_sync_recording(&self) {
        self.core.sync_recording_held.send_replace(None);
        self.core.sync_recording_released.send_replace(false);
        self.core.hold_sync_recording.store(true, Ordering::SeqCst);
    }

    pub async fn held_sync_request(&self) -> Value {
        let mut held = self.core.sync_recording_held.subscribe();
        tokio::time::timeout(WAIT, held.wait_for(Option::is_some))
            .await
            .expect("sync request must reach recording barrier")
            .expect("recording barrier must remain open")
            .clone()
            .unwrap()
    }

    pub fn release_sync_recording(&self) {
        self.core.sync_recording_released.send_replace(true);
    }

    pub fn sync_arrivals(&self) -> Vec<Value> {
        self.core.sync_requests.lock().unwrap().clone()
    }
}
