//! 只给测试用的内部状态辅助：直接拨连接状态，观测任务和界面怎么反应，隧道本身不动。

use wonton_protocol::HomeStatus;

use super::ClientCore;

impl ClientCore {
    /// 测试用：只改连接状态，隧道本身不动，观测任务和界面怎么反应。
    pub fn set_home_status_for_test(&self, status: HomeStatus) {
        // ui-fake 下 views 是 RwLock，写要走写锁；普通构建里就是字段本身。
        #[cfg(feature = "ui-fake")]
        self.views
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .home
            .send_modify(|view| view.status = status);
        #[cfg(not(feature = "ui-fake"))]
        self.views.home.send_modify(|view| view.status = status);
    }
}
