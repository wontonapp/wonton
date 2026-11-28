//! 假主面板：根页面是聊天，栈里钉着网页应用的标签页和一张加载得出来的本机网页。
//!
//! 面板里的那一页一直在，不跟着栈变；`fake_pages` 只搭初态，之后的开关由 `dispatch.rs` 照真核心的规则改。

use wonton_protocol::{BarEntry, PagesViewModel, PanelItem, PanelSource};

use super::super::ui_fake_room;
use super::tabs::{WEB_APPS, fake_page, web_app_tab};

pub(super) fn fake_pages(mut pages: PagesViewModel) -> PagesViewModel {
    pages.tab = BarEntry::Agent;
    pages.stack.clear();
    pages.my_app_candidates = vec![];
    // 主面板里钉着一页：聊天列表往下拉有东西可看，面板的出口才测得到。
    let (_, panel_app, _) = WEB_APPS[0];
    let tab = web_app_tab(panel_app);
    pages.panel = fake_page(&tab)
        .map(|page| PanelItem {
            tab,
            page,
            attention: false,
            source: PanelSource::User,
            movable: true,
        })
        .into_iter()
        // 第二页是加载得出来的本机网页，面板手势的走屏翻到它。
        .chain(ui_fake_room::panel())
        .collect();
    pages.panel_selected = None;
    pages
}
