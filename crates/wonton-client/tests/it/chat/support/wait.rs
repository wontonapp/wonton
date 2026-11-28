//! 手机视图的条件等待。

use std::time::Duration;

use wonton_client::{ChatAccount, ChatViewModel};

use super::Home;
use super::Phone;
use super::diagnostics::pending_requests;

/// 一次等待的上限。换服务器、隧道刚抖过时，旧会话的 `stop_sync` 可能先挂住几十秒，`wait_ready`/`signed_in` 要熬过
/// 这段窗口；CI 的 Docker 作业挤在几核里，登录比本机慢得多，60s 会先到。nextest 的 docker 档案 10 分钟内杀掉测试。
pub const WAIT: Duration = Duration::from_secs(300);
impl Phone {
    /// 等聊天状态满足条件。
    pub async fn wait_chat<R>(
        &self,
        what: &str,
        check: impl FnMut(&ChatViewModel) -> Option<R>,
    ) -> R {
        self.wait_chat_within(what, WAIT, check).await
    }
    pub async fn wait_chat_within<R>(
        &self,
        what: &str,
        within: Duration,
        mut check: impl FnMut(&ChatViewModel) -> Option<R>,
    ) -> R {
        let subscription = self.subscribe_chat();
        let since = std::time::Instant::now();
        let mut last = None;
        let waiting = async {
            while let Some(view) = subscription.next().await {
                if let Some(found) = check(&view) {
                    return found;
                }
                last = Some(view);
            }
            panic!("订阅提前结束");
        };
        match tokio::time::timeout(within, waiting).await {
            Ok(found) => found,
            Err(_) => {
                let view = self.chat();
                // 起步现场的记录按 Matrix 设备号分开，设备号只在登录后才有。
                let device = match &view.account {
                    ChatAccount::SignedIn { device_id, .. } => device_id.as_str(),
                    _ => "",
                };
                panic!(
                    "{what}：{within:?} 内没有等到，当前：{view:#?}\n同步起步：{}\n挂着的请求：{}",
                    wonton_matrix::last_sync_diagnostic(device),
                    pending_requests(since)
                )
            }
        }
    }
    /// 同 [`Self::wait_chat_within`]，超时额外带上最后一次视图和完整同步请求序列，供同步注入类测试诊断。
    pub async fn wait_chat_within_and_sync<R>(
        &self,
        home: &Home,
        what: &str,
        within: Duration,
        mut check: impl FnMut(&ChatViewModel) -> Option<R>,
    ) -> R {
        let subscription = self.subscribe_chat();
        let since = std::time::Instant::now();
        let mut last = None;
        let waiting = async {
            while let Some(view) = subscription.next().await {
                if let Some(found) = check(&view) {
                    return found;
                }
                last = Some(view);
            }
            panic!("订阅提前结束");
        };
        match tokio::time::timeout(within, waiting).await {
            Ok(found) => found,
            Err(_) => {
                let view = self.chat();
                let device = match &view.account {
                    ChatAccount::SignedIn { device_id, .. } => device_id.as_str(),
                    _ => "",
                };
                panic!(
                    "{what}：{within:?} 内没有等到，当前：{view:#?}\n最后一次视图：{last:#?}\n同步起步：{}\n同步请求：{:#?}\n挂着的请求：{}",
                    wonton_matrix::last_sync_diagnostic(device),
                    home.sync_requests(),
                    pending_requests(since)
                )
            }
        }
    }
}
