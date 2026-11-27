//! 假页面：根页面是聊天，栈是空的；`fake_pages` 只搭初态，之后的开关由 `dispatch.rs` 照真核心的规则改。

use wonton_protocol::{BarEntry, PagesViewModel, PanelItem, PanelSource};

use super::super::ui_fake_room;
use super::tabs::{WEB_APPS, fake_page, web_app_tab};

pub(super) fn fake_pages(mut pages: PagesViewModel) -> PagesViewModel {
    pages.tab = BarEntry::Agent;
    pages.stack.clear();
    pages.my_app_candidates = vec![];
    pages
}
