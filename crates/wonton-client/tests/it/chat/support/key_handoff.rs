//! 设备密钥上传和钥匙交接的开关（ADR-0046）。

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use tokio::sync::watch;

use super::Home;

impl Home {
    /// 让新手机要钥匙的请求比它发给 `device_id` 的确认完成消息早一轮同步到那台设备：
    /// 新手机先确认、旧手机后确认时会自然出现的顺序（ADR-0046）。
    pub fn deliver_key_request_before_done(&self, device_id: &str) {
        self.core.key_requests_forwarded.send_replace(0);
        *self.core.done_after_key_request.lock().unwrap() = Some(device_id.to_owned());
    }

    /// 要秘密的请求先不送到旧设备：`false` 时把留着的按原顺序补发。
    pub fn hold_key_requests(&self, hold: bool) {
        self.core.hold_key_requests.send_replace(hold);
    }

    pub fn drop_key_requests(&self, drop: bool) {
        self.core.drop_key_requests.store(drop, Ordering::SeqCst);
    }
    /// 停在上传请求体前，避免取消测试和上传完成竞跑。
    pub fn pause_uploads(&self) {
        self.core.uploads_paused.send_replace(true);
    }

    /// 回归测试开关：挂起期间手机的 keys/upload 一接上就断开（drop 语义，不挡其他请求；SDK 的
    /// outgoing 请求失败只记日志，下一轮同步自动重试）。
    pub fn hold_key_uploads(&self) {
        self.core.hold_key_uploads.store(true, Ordering::SeqCst);
    }

    pub fn delay_key_uploads(&self, delay: Duration) {
        *self.core.key_upload_delay.lock().unwrap() = delay;
    }

    /// 开关本体给放开挂起的任务：先到先放的逻辑里，放行的一方不持有 [`Home`]。
    pub fn key_uploads_hold(&self) -> Arc<AtomicBool> {
        self.core.hold_key_uploads.clone()
    }

    /// 完成计数的接收端：`changed()` 等下一次 keys/upload 收到 Tuwunel 的成功响应。
    pub fn key_uploads_completed(&self) -> watch::Receiver<u64> {
        self.core.key_uploads_completed.subscribe()
    }

    /// 等待函数进门时调用：放开的任务看到它就知道测试开始等设备密钥了。
    pub fn mark_device_keys_wait_started(&self) {
        self.core.device_keys_wait_started.send_replace(true);
    }

    pub fn device_keys_wait_started(&self) -> watch::Receiver<bool> {
        self.core.device_keys_wait_started.subscribe()
    }

    pub fn resume_uploads(&self) {
        self.core.uploads_paused.send_replace(false);
    }
}
