//! 假数据入口的单元测试：塞进去的 ViewModel 和操作不落到作用域、页面栈照真核心的规则开关、
//! 确认卡片回答后带上设备、我的应用详情、通话群的 widget 入口、旁聊发照片不死锁。

use std::sync::atomic::Ordering;

use super::super::ClientCore;
use wonton_protocol::{
    Action, AppsAction, BarEntry, BotChatAvailability, ComposerViewModel, HomeStatus, ImageUpload,
    MAX_BAR_ITEMS, MyAppRef, OnboardingStep, PageAction, TabRef, TabState, TimelineAction,
    TimelineTarget,
};
/// 假数据塞进了各订阅、操作不落到作用域：漏填字段或写坏类型在这里就炸。
#[test]
fn seeded_views_are_set_and_dispatch_is_inert() {
    let dir = std::env::temp_dir().join(format!("wonton-ui-fake-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let core = ClientCore::new_ui_fake(dir.to_string_lossy().into_owned()).unwrap();
    assert!(core.ui_fake.load(Ordering::SeqCst));
    assert_eq!(core.home().status, HomeStatus::Connected);
    assert_eq!(core.servers().servers.len(), 1);
    assert_eq!(core.chat().rooms[0].room_id, ROOM_ID);
    assert!(core.chat().rooms[1].call_active);
    // 私聊对象可用：夜跑在 Echo 私聊里点发送，不能因为资格未知被禁用。
    assert!(
        core.chat()
            .bots
            .iter()
            .all(|bot| bot.availability == BotChatAvailability::Available)
    );
    assert_eq!(core.pages().tab, BarEntry::Agent);
    assert_eq!(core.onboarding().step, Some(OnboardingStep::Finished));
    assert_eq!(core.plugins().installed.unwrap().plugins[0].title, "Notes");
    // 作用域的首帧是后台算完才发的：等一会儿，确认它盖不掉假数据。
    std::thread::sleep(std::time::Duration::from_millis(500));
    assert_eq!(core.home().status, HomeStatus::Connected);
    assert_eq!(core.chat().rooms[0].name, "Echo");
    assert_eq!(
        core.plugins().actions,
        [AppsAction::Store, AppsAction::AddSource]
    );
    assert_eq!(core.pages().tab, BarEntry::Agent);
    assert_eq!(core.onboarding().step, Some(OnboardingStep::Finished));
    // 操作不落到作用域。
    assert!(
        core.dispatch(Action::Pages {
            action: PageAction::SelectBarItem {
                entry: BarEntry::Apps,
            },
        })
        .is_ok()
    );
    assert!(
        core.dispatch(Action::Plugins {
            action: wonton_protocol::PluginAction::RefreshPlugins
        })
        .is_ok()
    );
    assert_eq!(core.pages().tab, BarEntry::Apps);
    let _ = std::fs::remove_dir_all(&dir);
}

async fn tray(core: &ClientCore) -> ComposerViewModel {
    let target = TimelineTarget {
        room_id: ROOM_ID.into(),
        thread_root: None,
    };
    let subscription = core.subscribe_composer(target).unwrap();
    subscription.next().await.unwrap()
}

/// 附件区在假数据入口下也按真逻辑加、拿：界面测试靠它走完相册加附件的路。
#[tokio::test]
async fn attachments_go_in_and_out_offline() {
    let dir = std::env::temp_dir().join(format!("wonton-ui-fake-tray-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let core = ClientCore::new_ui_fake(dir.to_string_lossy().into_owned()).unwrap();
    let target = TimelineTarget {
        room_id: ROOM_ID.into(),
        thread_root: None,
    };
    let photo = || ImageUpload {
        filename: "photo.png".into(),
        mime_type: "image/png".into(),
        data: vec![1, 2, 3],
        width: Some(4),
        height: Some(3),
        thumbnail: None,
    };
    let first = core.add_image_attachment(target.clone(), photo()).unwrap();
    let second = core.add_image_attachment(target.clone(), photo()).unwrap();
    assert_eq!(tray(&core).await.attachments.len(), 2);
    core.dispatch(Action::Timeline {
        action: TimelineAction::RemoveAttachment {
            target: target.clone(),
            attachment_id: first.clone(),
        },
    })
    .unwrap();
    let view = tray(&core).await;
    assert_eq!(view.attachments.len(), 1);
    assert_eq!(view.attachments[0].id, second);
    // 拿掉的再拿一次才算找不到。
    assert!(
        core.dispatch(Action::Timeline {
            action: TimelineAction::RemoveAttachment {
                target,
                attachment_id: first,
            },
        })
        .is_err()
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// 开页、关页照真核心的规则走：压栈、同一页只留一份并挪到栈顶、关页弹栈、根页切换清栈、底栏增删移。
#[test]
fn pages_open_and_close_like_the_real_stack() {
    let dir = std::env::temp_dir().join(format!("wonton-ui-fake-pages-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let core = ClientCore::new_ui_fake(dir.to_string_lossy().into_owned()).unwrap();
    let open = |id: &str| {
        core.dispatch(Action::Pages {
            action: PageAction::OpenPage {
                tab: web_app_tab(id),
            },
        })
    };
    let select = |entry: BarEntry| {
        core.dispatch(Action::Pages {
            action: PageAction::SelectBarItem { entry },
        })
    };
    let stack = || -> Vec<TabRef> {
        core.pages()
            .stack
            .into_iter()
            .map(|open| open.tab)
            .collect()
    };
    assert!(stack().is_empty());
    assert_eq!(core.pages().bar.items.len(), 2);

    open("app.example.budget").unwrap();
    open("app.example.photos").unwrap();
    open("app.example.budget").unwrap();
    assert_eq!(
        stack(),
        [
            web_app_tab("app.example.photos"),
            web_app_tab("app.example.budget")
        ]
    );
    assert_eq!(core.pages().stack[0].page.state, TabState::Stopped);

    core.dispatch(Action::Pages {
        action: PageAction::ClosePage,
    })
    .unwrap();
    assert_eq!(stack(), [web_app_tab("app.example.photos")]);
    // 根页切换一路弹回根；栈空时再关一次是安静的什么也不做。
    select(BarEntry::Agent).unwrap();
    assert!(stack().is_empty());
    core.dispatch(Action::Pages {
        action: PageAction::ClosePage,
    })
    .unwrap();
    assert!(stack().is_empty());
    select(BarEntry::Apps).unwrap();
    assert_eq!(core.pages().tab, BarEntry::Apps);
    open("app.example.budget").unwrap();
    assert_eq!(core.pages().tab, BarEntry::Apps);
    assert_eq!(stack(), [web_app_tab("app.example.budget")]);

    // 收件箱是压栈的一页，重复打开只留一份并挪到栈顶。
    core.dispatch(Action::Pages {
        action: PageAction::OpenPage { tab: TabRef::Inbox },
    })
    .unwrap();
    assert_eq!(stack(), [web_app_tab("app.example.budget"), TabRef::Inbox]);
    core.dispatch(Action::Pages {
        action: PageAction::OpenPage { tab: TabRef::Inbox },
    })
    .unwrap();
    assert_eq!(stack(), [web_app_tab("app.example.budget"), TabRef::Inbox]);
    select(BarEntry::Apps).unwrap();
    assert!(stack().is_empty());

    // 底栏：加一项、重复拒、满了记 full、删、挪。
    let budget = MyAppRef::Page {
        tab: web_app_tab("app.example.budget"),
    };
    core.dispatch(Action::Pages {
        action: PageAction::AddBarItem {
            app: budget.clone(),
        },
    })
    .unwrap();
    assert_eq!(core.pages().bar.items.len(), 3);
    assert!(
        core.dispatch(Action::Pages {
            action: PageAction::AddBarItem {
                app: budget.clone()
            },
        })
        .is_err()
    );
    for id in [
        "app.example.photos",
        "app.example.notes",
        "app.example.calendar",
    ] {
        core.dispatch(Action::Pages {
            action: PageAction::AddBarItem {
                app: MyAppRef::Page {
                    tab: web_app_tab(id),
                },
            },
        })
        .unwrap();
    }
    assert_eq!(core.pages().bar.items.len(), MAX_BAR_ITEMS as usize);
    assert!(
        core.dispatch(Action::Pages {
            action: PageAction::AddBarItem {
                app: MyAppRef::Plugin {
                    plugin_id: "app.example.one-more".into(),
                },
            },
        })
        .is_err()
    );
    assert!(core.pages().bar.full);
    core.dispatch(Action::Pages {
        action: PageAction::MoveBarItem { from: 0, to: 2 },
    })
    .unwrap();
    assert_eq!(core.pages().bar.items[2].entry, BarEntry::Agent);
    core.dispatch(Action::Pages {
        action: PageAction::RemoveBarItem {
            entry: BarEntry::Agent,
        },
    })
    .unwrap();
    assert!(!core.pages().bar.full);
    assert_eq!(core.pages().bar.items.len(), MAX_BAR_ITEMS as usize - 1);
    while core.pages().bar.items.len() > 1 {
        let entry = core.pages().bar.items[0].entry.clone();
        core.dispatch(Action::Pages {
            action: PageAction::RemoveBarItem { entry },
        })
        .unwrap();
    }
    assert!(
        core.dispatch(Action::Pages {
            action: PageAction::RemoveBarItem {
                entry: core.pages().bar.items[0].entry.clone(),
            },
        })
        .is_err()
    );

    // 整页打开本机网页的那个应用：压进栈的是加载得出来的那一页。
    open(CORNER_APP).unwrap();
    let top = core.pages().stack.pop().unwrap();
    assert!(matches!(top.tab, TabRef::Url { .. }));
    assert_eq!(top.page.state, TabState::Ready);
    // 面板里钉着的那一页一直在，不跟着栈变。
    assert_eq!(core.pages().panel.len(), 2);
    let _ = std::fs::remove_dir_all(&dir);
}

/// 带设备的安装卡片：同意以后卡片结束，结果里是随回答带上的设备 ID。
#[test]
fn answering_the_device_card_reports_the_devices_sent() {
    let dir = std::env::temp_dir().join(format!("wonton-ui-fake-devices-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let core = ClientCore::new_ui_fake(dir.to_string_lossy().into_owned()).unwrap();
    let card = core.confirmations().pending.remove(0).card;
    assert!(card.devices.iter().all(|device| !device.chosen));
    let devices = crate::toggle_device(
        card.devices,
        "disk:/dev/disk/by-id/ata-WDC_WD40EFRX-68N32N0".into(),
    );
    core.dispatch(Action::Chat {
        action: wonton_protocol::ChatAction::AnswerConfirmation {
            id: card.id,
            answer: wonton_protocol::Answer::Approve,
            choice_id: None,
            secrets: Default::default(),
            settings: Default::default(),
            devices: crate::chosen_device_ids(devices, card.permissions),
        },
    })
    .unwrap();
    let view = core.confirmations();
    assert!(view.pending.is_empty());
    assert_eq!(
        view.finished[0].card.state,
        wonton_protocol::ConfirmationState::Succeeded {
            message: "disk:/dev/disk/by-id/ata-WDC_WD40EFRX-68N32N0".into()
        }
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// 设置里的“我的应用”要能点开详情看分享：行里有两个插件，主人在假数据里是主人，
/// 一个应用已经分享给一位好友、另一个一个都没有。
#[test]
fn my_apps_rows_open_details_with_share_state() {
    let dir = std::env::temp_dir().join(format!("wonton-ui-fake-apps-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let core = ClientCore::new_ui_fake(dir.to_string_lossy().into_owned()).unwrap();
    let rows = core.plugins().sidebar.apps;
    assert_eq!(
        rows.iter()
            .filter_map(|app| app.plugin_id.as_deref())
            .collect::<Vec<_>>(),
        [
            ui_fake_plugin_detail::BUDGET_ID,
            ui_fake_plugin_detail::HUB_ID,
        ]
    );
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    assert!(runtime.block_on(core.me()).unwrap().is_owner);
    let detail = |id: &str| {
        runtime
            .block_on(
                core.plugin_detail(wonton_protocol::PluginDetailRoute::Installed { id: id.into() }),
            )
            .unwrap()
    };
    for (id, friends) in [
        (ui_fake_plugin_detail::HUB_ID, 1),
        (ui_fake_plugin_detail::BUDGET_ID, 0),
    ] {
        let wonton_protocol::PluginDetailOutcome::Detail { detail } = detail(id) else {
            panic!("{id} must have a fake detail");
        };
        assert_eq!(detail.shares.unwrap().friend_count, friends);
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// 通话中的群聊：时间线带 widget 入口和“通话中”，入口的页面开得出来、关得回去；别的房间没有入口。
#[test]
fn the_call_room_offers_a_widget_page() {
    let dir = std::env::temp_dir().join(format!("wonton-ui-fake-call-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let core = ClientCore::new_ui_fake(dir.to_string_lossy().into_owned()).unwrap();
    // 核心自己带运行时，不能在异步上下文里释放：只有取时间线这两步放进一个临时的运行时。
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    let first = |room_id: &str| {
        let target = wonton_protocol::TimelineTarget {
            room_id: room_id.into(),
            thread_root: None,
        };
        runtime.block_on(async {
            let timeline = core.subscribe_timeline(target).await.unwrap();
            timeline.next().await.unwrap()
        })
    };
    let quiet = first(ROOM_ID);
    assert!(quiet.widgets.is_empty() && !quiet.call_active);
    let call = first(CALL_ROOM_ID);
    assert!(call.call_active);
    let [entry] = call.widgets.as_slice() else {
        panic!("the call room must have exactly one widget entry");
    };

    core.dispatch(Action::Pages {
        action: PageAction::OpenPage {
            tab: entry.tab.clone(),
        },
    })
    .unwrap();
    let pages = core.pages();
    assert_eq!(pages.stack.len(), 1);
    assert_eq!(pages.stack[0].tab, entry.tab);
    assert_eq!(pages.stack[0].page.state, TabState::Ready);
    core.dispatch(Action::Pages {
        action: PageAction::ClosePage,
    })
    .unwrap();
    assert!(core.pages().stack.is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

/// 旁聊里带回复发照片不会卡住：推送时状态锁和订阅名单的锁都已放掉，壳就地报滚动也能绕回来。
#[tokio::test]
async fn side_chat_photo_with_reply_does_not_block() {
    use tokio::time::{Duration, timeout};
    use wonton_protocol::TimelineDiff;
    let dir = std::env::temp_dir().join(format!("wonton-ui-fake-photo-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let core = ClientCore::new_ui_fake(dir.to_string_lossy().into_owned()).unwrap();
    let target = TimelineTarget {
        room_id: ROOM_ID.into(),
        thread_root: Some("$root".into()),
    };
    let sub = core.subscribe_timeline(target.clone()).await.unwrap();
    let first = sub.next().await.unwrap();
    assert!(!first.diffs.is_empty());
    core.dispatch(Action::Timeline {
        action: TimelineAction::SendText {
            target: target.clone(),
            body: "回这一条".into(),
            reply_to: None,
            mentions: Vec::new(),
        },
    })
    .unwrap();
    let update = timeout(Duration::from_secs(5), sub.next())
        .await
        .unwrap()
        .unwrap();
    let id = update
        .diffs
        .iter()
        .find_map(|diff| match diff {
            TimelineDiff::PushBack { item } => Some(item.id.clone()),
            _ => None,
        })
        .expect("没拿到刚发的 id");
    core.dispatch(Action::Timeline {
        action: TimelineAction::SendImage {
            target,
            image: ImageUpload {
                filename: "photo.png".into(),
                mime_type: "image/png".into(),
                data: vec![1, 2, 3],
                width: Some(4),
                height: Some(3),
                thumbnail: None,
            },
            reply_to: Some(format!("${id}")),
            caption: Some("说明".into()),
        },
    })
    .unwrap();
    timeout(Duration::from_secs(5), sub.next())
        .await
        .expect("send_image 卡住了");
    let _ = std::fs::remove_dir_all(&dir);
}
