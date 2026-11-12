use wonton_protocol::CoreError;

use super::ClientCore;

#[uniffi::export]
impl ClientCore {
    /// `TabPage.opens_in_browser` 的页面在系统浏览器里打开用的地址；`url` 是那个页面的 `TabPage.url`。
    /// 地址只在本机回环上有效，只交给浏览器（Android 用 Custom Tabs），不写日志、不分享（ADR-0033）。
    pub fn plugin_browser_url(&self, url: String) -> Result<String, CoreError> {
        self.scope().browser.open(&url)
    }
}
