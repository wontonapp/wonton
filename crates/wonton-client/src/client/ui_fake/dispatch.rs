//! 假数据入口的操作分发：只照真核心的规则改页面栈和确认卡片，其余 ViewModel 不动。

use std::sync::atomic::{AtomicUsize, Ordering};

use wonton_protocol::{
    AskFailure, AskProblem, AskReceipt, AskState, BarEntry, BarIcon, BarItem, ChatAction,
    CoreError, MAX_BAR_ITEMS, OpenPage, PageAction, PanelItem, PanelScope, PanelSource, TabIcon,
    TabLayout, TabPage, TabRef, TabState,
};

use super::super::ClientCore;
use super::tabs::fake_open;

/// 假数据下问过几次 Bot；`WONTON_UI_FAKE_ASK=contact_required_once` 用它只让第一次失败。
static UI_FAKE_ASKS: AtomicUsize = AtomicUsize::new(0);

impl ClientCore {
    /// 假数据下的操作：开页、关页照真核心的规则改页面栈（`pages::actions`），其余不动 ViewModel。
    #[allow(clippy::too_many_lines)] // 假数据把真核心的各条规则摆在一处，拆开要来回传同一份视图。
    pub(in crate::client) fn dispatch_ui_fake(&self, action: PageAction) -> Result<(), CoreError> {
        let views = self.views();
        match action {
            PageAction::OpenPage { tab } => {
                if matches!(tab, TabRef::Rooms) {
                    views.pages.send_modify(|pages| pages.stack.clear());
                    return Ok(());
                }
                if matches!(tab, TabRef::Inbox) {
                    views.pages.send_modify(|pages| {
                        if pages.stack.last().is_some_and(|open| open.tab == tab) {
                            return;
                        }
                        pages.stack.retain(|open| open.tab != tab);
                        pages.stack.push(fake_root_page(tab.clone()));
                    });
                    return Ok(());
                }
                let Some((tab, page)) = fake_open(tab.clone()) else {
                    views.pages.send_modify(|pages| {
                        pages.stack.clear();
                        if let BarEntry::App { .. } = pages.tab {
                            pages.tab = BarEntry::Agent;
                        }
                    });
                    return Ok(());
                };
                views.pages.send_modify(|pages| {
                    if pages.stack.last().is_some_and(|open| open.tab == tab) {
                        return;
                    }
                    pages.stack.retain(|open| open.tab != tab);
                    pages.stack.push(OpenPage { tab, page });
                });
                Ok(())
            }
            PageAction::ClosePage => {
                // 和真核心一样：栈空时的“关”从应用根回 Agent。
                views.pages.send_modify(|pages| {
                    if !pages.stack.pop().is_some()
                        && pages.tab == BarEntry::Apps
                        && pages
                            .bar
                            .items
                            .iter()
                            .any(|item| item.entry == BarEntry::Agent)
                    {
                        pages.tab = BarEntry::Agent;
                    }
                });
                Ok(())
            }
            PageAction::SelectBarItem { entry } => {
                let mut refused = false;
                views.pages.send_modify(|pages| match &entry {
                    BarEntry::Agent | BarEntry::Apps => {
                        pages.tab = entry.clone();
                        pages.stack.clear();
                    }
                    BarEntry::App { app } => {
                        if !pages.bar.items.iter().any(|item| item.entry == entry) {
                            refused = true;
                            return;
                        }
                        let tab = match app {
                            wonton_protocol::MyAppRef::Page { tab } => tab.clone(),
                            wonton_protocol::MyAppRef::Plugin { plugin_id } => TabRef::Plugin {
                                plugin_id: plugin_id.clone(),
                                page: None,
                            },
                        };
                        if let Some((tab, page)) = fake_open(tab.clone()) {
                            pages.stack.retain(|open| open.tab != tab);
                            pages.stack.push(OpenPage { tab, page });
                        }
                    }
                });
                if refused {
                    Err(CoreError::InvalidInput {
                        reason: "fake bar item is not pinned".into(),
                    })
                } else {
                    Ok(())
                }
            }
            PageAction::AddBarItem { entry } => {
                let mut added = true;
                views.pages.send_modify(|pages| {
                    let known = match &entry {
                        BarEntry::Agent | BarEntry::Apps => true,
                        BarEntry::App {
                            app: wonton_protocol::MyAppRef::Page { tab },
                        } => fake_open(tab.clone()).is_some(),
                        BarEntry::App {
                            app: wonton_protocol::MyAppRef::Plugin { plugin_id },
                        } => fake_open(TabRef::Plugin {
                            plugin_id: plugin_id.clone(),
                            page: None,
                        })
                        .is_some(),
                    };
                    if !known || pages.bar.items.iter().any(|item| item.entry == entry) {
                        added = false;
                        return;
                    }
                    if pages.bar.items.len() >= MAX_BAR_ITEMS as usize {
                        pages.bar.full = true;
                        added = false;
                        return;
                    }
                    pages.bar.full = false;
                    pages.bar.items.push(BarItem {
                        entry,
                        label: "App".into(),
                        icon: BarIcon::App { icon: None },
                        badge: 0,
                    });
                });
                if added {
                    Ok(())
                } else {
                    Err(CoreError::InvalidInput {
                        reason: "fake bar refused".into(),
                    })
                }
            }
            PageAction::RemoveBarItem { entry } => {
                let mut refused = false;
                views.pages.send_modify(|pages| {
                    if !pages.bar.items.iter().any(|item| item.entry == entry)
                        || pages.bar.items.len() <= 1
                    {
                        refused = true;
                    } else {
                        pages.bar.items.retain(|item| item.entry != entry);
                        pages.bar.full = false;
                    }
                });
                if refused {
                    Err(CoreError::InvalidInput {
                        reason: "fake bar refused".into(),
                    })
                } else {
                    Ok(())
                }
            }
            PageAction::MoveBarItem { from, to } => {
                let mut refused = false;
                views.pages.send_modify(|pages| {
                    let len = pages.bar.items.len();
                    let (from, to) = (from as usize, to as usize);
                    if from >= len || to >= len {
                        refused = true;
                        return;
                    }
                    let item = pages.bar.items.remove(from);
                    pages.bar.items.insert(to, item);
                });
                if refused {
                    Err(CoreError::InvalidInput {
                        reason: "fake bar index out of range".into(),
                    })
                } else {
                    Ok(())
                }
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
                if mode == "silent" {
                    return;
                }
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

/// 假收件箱页：壳按 `TabRef::Inbox` 画自己的原生列表，元数据只给胶囊用。
fn fake_root_page(tab: TabRef) -> OpenPage {
    OpenPage {
        tab,
        page: TabPage {
            title: "Inbox".into(),
            icon: TabIcon::Plugin { icon: None },
            url: None,
            layout: TabLayout::Inset,
            state: TabState::Ready,
            reload: 0,
            opens_in_browser: false,
        },
    }
}
