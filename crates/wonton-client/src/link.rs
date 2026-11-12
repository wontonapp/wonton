//! App 链接的编解码给壳用：格式在 [`AppLink`] 里，两端不自己拼。

use wonton_protocol::AppLink;

/// 桌面快捷方式、快捷指令用的 `wonton://` 链接。
#[uniffi::export]
pub fn app_link_url(link: AppLink) -> String {
    link.to_url()
}

/// 认不出的链接是空的：壳照常打开 App，不报错。
#[uniffi::export]
pub fn parse_app_link(url: String) -> Option<AppLink> {
    AppLink::parse(&url)
}
