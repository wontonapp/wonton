//! 假数据入口的操作分发：只照真核心的规则改页面栈和确认卡片，其余 ViewModel 不动。

use wonton_protocol::{ChatAction, CoreError, OpenPage, PageAction, TabRef};

use super::super::ClientCore;
use super::tabs::fake_open;

impl ClientCore {
    /// 假数据下的操作：开页、关页照真核心的规则改页面栈（`pages::actions`），其余不动 ViewModel。
    pub(in crate::client) fn dispatch_ui_fake(&self, action: PageAction) -> Result<(), CoreError> {
        let views = self.views();
        match action {
            PageAction::OpenPage { tab } => {
                let Some((tab, page)) = fake_open(tab.clone()) else {
                    // 根页面（聊天、应用）一路弹回根；应用网格开没开记在页面视图里。
                    views.pages.send_modify(|pages| {
                        pages.stack.clear();
                        pages.shows_apps = tab == TabRef::Apps;
                    });
                    return Ok(());
                };
                views.pages.send_if_modified(|pages| {
                    let left_apps = std::mem::take(&mut pages.shows_apps);
                    if pages.stack.last().is_some_and(|open| open.tab == tab) {
                        return left_apps;
                    }
                    pages.stack.retain(|open| open.tab != tab);
                    pages.stack.push(OpenPage { tab, page });
                    true
                });
                Ok(())
            }
            PageAction::ClosePage => {
                // 和真核心一样：没有开着的页时是安静的什么也不做。
                views.pages.send_if_modified(|pages| {
                    // 应用网格开着时关的是它，栈不动。
                    std::mem::take(&mut pages.shows_apps) || pages.stack.pop().is_some()
                });
                Ok(())
            }
            _ => Ok(()),
        }
    }

    pub(in crate::client) fn dispatch_ui_fake_chat(&self, action: ChatAction) {
        if let ChatAction::AnswerConfirmation {
            id,
            answer,
            devices,
            ..
        } = action
        {
            self.views().confirmations.send_if_modified(|view| {
                super::super::ui_fake_devices::answer(view, &id, answer, &devices)
            });
        }
    }
}
