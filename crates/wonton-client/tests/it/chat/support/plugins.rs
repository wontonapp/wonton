//! 已装插件和本人私人 Bot 的假应答。

use serde_json::Value;
use std::sync::atomic::Ordering;
use wonton_client::{InstalledPlugin, InstalledPlugins};

use super::Home;

impl Home {
    /// 设置假核心的“本人私人 Bot”应答；没设置时这个接口回 404，客户端显示读取问题。
    pub fn set_own_private_bot(&self, own_private_bot: Value) {
        *self.core.own_private_bot.lock().unwrap() = Some(own_private_bot);
    }

    /// 设置假核心的已装列表应答；没设置时这个接口回 404。
    pub fn set_installed_plugins(&self, plugins: Vec<InstalledPlugin>) {
        let installed = serde_json::to_value(InstalledPlugins {
            plugins,
            upgrade_check: None,
        })
        .unwrap();
        *self.core.installed.lock().unwrap() = Some(installed);
    }

    /// 取已装列表的请求次数，数的是这个测试会话里的全部。
    pub fn installed_requests(&self) -> usize {
        self.core.installed_requests.load(Ordering::SeqCst)
    }
}
