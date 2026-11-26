//! 假数据入口的操作分发：只照真核心的规则改页面栈和确认卡片，其余 ViewModel 不动。

use std::sync::atomic::{AtomicUsize, Ordering};

use wonton_protocol::{
    AskFailure, AskProblem, AskReceipt, AskState, ChatAction, CoreError, OpenPage, PageAction,
    PanelItem, PanelScope, PanelSource, TabRef,
};
};
use super::super::ClientCore;
use super::tabs::fake_open;

/// 假数据下问过几次 Bot；`WONTON_UI_FAKE_ASK=contact_required_once` 用它只让第一次失败。
static UI_FAKE_ASKS: AtomicUsize = AtomicUsize::new(0);

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
            // 面板的增删和选择在真核心写账号数据；假数据把同一份视图直接改掉，面板走屏不用后端。
            // 只动主面板：房间面板的页来自时间线，假数据里没有对应的视图可改。
            PageAction::AddPanelPage { scope, tab } => {
                if scope != PanelScope::Main {
                    return Ok(());
                }
                let Some(page) = super::tabs::fake_page(&tab) else {
                    return Ok(());
                };
                views.pages.send_modify(|pages| {
                    if pages.panel.iter().any(|item| item.tab == tab) {
                        return;
                    }
                    pages.panel.push(PanelItem {
                        tab,
                        page,
                        attention: false,
                        source: PanelSource::User,
                        movable: true,
                    });
                });
                Ok(())
            }
            PageAction::RemovePanelPage { scope, tab } => {
                if scope != PanelScope::Main {
                    return Ok(());
                }
                views.pages.send_modify(|pages| {
                    pages.panel.retain(|item| item.tab != tab);
                    if pages.panel_selected.as_ref() == Some(&tab) {
                        pages.panel_selected = None;
                    }
                });
                Ok(())
            }
            PageAction::SelectPanelPage { scope, tab } => {
                if scope != PanelScope::Main {
                    return Ok(());
                }
                views
                    .pages
                    .send_modify(|pages| pages.panel_selected = Some(tab));
                Ok(())
            }
            _ => Ok(()),
        }
    }

    pub(in crate::client) fn dispatch_ui_fake_chat(&self, action: ChatAction) {
        match action {
            ChatAction::AnswerConfirmation {
                id,
                answer,
                devices,
                ..
            } => {
                self.views().confirmations.send_if_modified(|view| {
                    super::super::ui_fake_devices::answer(view, &id, answer, &devices)
                });
            }
            // “问 Bot”在假数据下立即成败：不给真实网络，就能测失败提示和重试。
            ChatAction::AskAgent {
                room_id,
                request_id,
                ..
            } => {
                let attempt = UI_FAKE_ASKS.fetch_add(1, Ordering::SeqCst);
                let mode = std::env::var("WONTON_UI_FAKE_ASK").unwrap_or_default();
                let fail =
                    mode == "contact_required" || (mode == "contact_required_once" && attempt == 0);
                let room = room_id.or_else(|| Some(super::chat::ROOM_ID.to_owned()));
                self.views().chat.send_modify(|view| {
                    view.ask = Some(AskReceipt {
                        request_id,
                        room_id: room,
                        state: if fail {
                            AskState::Failed
                        } else {
                            AskState::Sent
                        },
                        problem: fail.then(|| AskProblem {
                            reason: AskFailure::ContactRequired,
                            detail: "Fake contact required".to_owned(),
                        }),
                    });
                });
            }
            _ => {}
        }
    }
}
