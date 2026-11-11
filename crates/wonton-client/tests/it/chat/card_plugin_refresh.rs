//! 确认卡片在本会话里从没结束变成"执行完了"时，已装列表多半还是旧的，卡片上的按钮凭它显示
//! （ADR-0016）：客户端要自己补取一次。启动时翻回来的历史结果不补取。测试扮演核心账号发卡片、
//! 编辑卡片，数已装列表的请求。

use std::time::{Duration, Instant};

use serde_json::{Value, json};
use wonton_client::{
    Action, Answer, ConfirmationState, InstalledPlugin, PluginHealth, PluginKind, PluginState,
    PluginTrust, PluginsViewModel, TabDisplay,
};

use super::confirmation::{
    answer, card, content, core_room, edit, in_ten_minutes, send, until_card, with_state,
};
use super::support::{Home, Phone, Timeline, WAIT};

const PLUGIN: &str = "app.grafana";

/// 钩子报到之后、放行之前，给 settled 的观察窗口：修复前 settled 在这窗口里出现，修复后一直不出现。
/// 实际的先后由钩子和决定的就地性决定，窗口只是给“出现”留够时间，不靠它排先后。
const HELD: Duration = Duration::from_secs(2);

fn install_card(id: &str) -> Value {
    let mut card = card(id, in_ten_minutes());
    card["plugin_id"] = json!(PLUGIN);
    card
}

fn succeeded(message: &str) -> Value {
    json!({"type": "succeeded", "message": message})
}

fn plugin() -> InstalledPlugin {
    InstalledPlugin {
        chat: Default::default(),
        id: PLUGIN.to_owned(),
        title: "看板".to_owned(),
        summary: String::new(),
        version: "1.0.0".to_owned(),
        revision: "1.0.0".to_owned(),
        kind: PluginKind::App,
        runtime: Default::default(),
        icon: None,
        trust: PluginTrust::Official,
        source: None,
        provider: None,
        via_index: None,
        written_by: None,
        status: PluginState::Running {
            health: PluginHealth::Healthy,
        },
        web_url: None,
        web_display: TabDisplay::default(),
        cross_origin_isolated: false,
        declares_tools: false,
        tabs: Vec::new(),
        bot_user_id: None,
        family: false,
        upgrade: None,
        removable: true,
        notices: Vec::new(),
        enabled_root_permissions: Vec::new(),
        install_progress: None,
        tunnel_ports: Vec::new(),
    }
}

async fn wait_for<R>(
    phone: &Phone,
    what: &str,
    check: impl Fn(&PluginsViewModel) -> Option<R>,
) -> R {
    let deadline = Instant::now() + WAIT;
    loop {
        if let Some(found) = check(&phone.plugins()) {
            return found;
        }
        assert!(
            Instant::now() < deadline,
            "{what}：{WAIT:?} 内没有等到，当前：{:#?}",
            phone.plugins()
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// 跟着核心私聊的那一路（补取靠它）和打开着的时间线各自订阅，可能晚一步：它把卡片列进等回答的，才算看见了没结束。
async fn followed_pending(phone: &Phone, id: &str) {
    phone
        .wait_chat("跟随看见卡片在等回答", |view| {
            view.pending_confirmations
                .iter()
                .any(|card| card.id == id)
                .then_some(())
        })
        .await;
}

/// 跟随把卡片列进不用回答的，补不补取就已经定了。
async fn followed_settled(phone: &Phone, id: &str) {
    phone
        .wait_chat("跟随看见卡片有了结果", |view| {
            view.settled_confirmations
                .iter()
                .any(|card| card == id)
                .then_some(())
        })
        .await;
}

/// settled_confirmations 里出现这张卡片。不带超时：由调用方决定等多久。
/// 先订阅再查现状，不然订阅前的更新会被当成“已经看过”漏掉。
async fn settled_seen(phone: &Phone, id: &str) {
    let subscription = phone.subscribe_chat();
    if phone
        .chat()
        .settled_confirmations
        .iter()
        .any(|card| card == id)
    {
        return;
    }
    while let Some(view) = subscription.next().await {
        if view.settled_confirmations.iter().any(|card| card == id) {
            return;
        }
    }
    panic!("聊天订阅提前结束");
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn a_succeeded_card_fetches_the_installed_list_once() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let home = Home::start(&[phone.device_id()]).await;
    // 列表一开始就没有这个插件：连上后的第一次取列表就是基线。
    home.set_installed_plugins(vec![]);
    phone.connect(&home);
    phone.signed_in().await;
    let (core, room_id) = core_room(&phone, &home).await;
    let room = core.open_room(&room_id).await.expect("核心打开私聊");
    let mut timeline = Timeline::open(&phone, room_id.as_str()).await;
    wait_for(&phone, "连上后的第一次取列表", |view| {
        view.installed.is_some().then_some(())
    })
    .await;
    let baseline = home.installed_requests();

    // 批准后核心把卡片编辑成"执行完了"：客户端自己补取一次，ViewModel 里就有了。
    let card = install_card("install");
    let posted = send(&room, content(&card)).await;
    until_card(&mut timeline, "install", "卡片出现", |view| {
        view.card.plugin_id.as_deref() == Some(PLUGIN)
    })
    .await;
    answer(&phone, "install", Answer::Approve);
    until_card(&mut timeline, "install", "收下了回答", |view| {
        view.card.state != ConfirmationState::Pending
    })
    .await;
    home.set_installed_plugins(vec![plugin()]);
    edit(
        &room,
        &posted,
        &with_state(&card, succeeded("装好了 Grafana")),
    )
    .await;
    until_card(&mut timeline, "install", "执行完了", |view| {
        matches!(view.card.state, ConfirmationState::Succeeded { .. })
    })
    .await;
    wait_for(&phone, "补取后的列表", |view| {
        view.installed
            .as_ref()
            .is_some_and(|installed| installed.plugins.iter().any(|plugin| plugin.id == PLUGIN))
            .then_some(())
    })
    .await;
    assert_eq!(home.installed_requests(), baseline + 1);

    // 同一张卡片再编辑一次（历史重放、重复到达的形状）：不补取。
    edit(
        &room,
        &posted,
        &with_state(&card, succeeded("装好了 Grafana 2")),
    )
    .await;
    until_card(&mut timeline, "install", "重放后的结果", |view| {
        view.card.state
            == ConfirmationState::Succeeded {
                message: "装好了 Grafana 2".to_owned(),
            }
    })
    .await;
    assert_eq!(home.installed_requests(), baseline + 1);
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn listed_plugins_do_not_refetch_and_missing_ones_only_fetch_once() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let home = Home::start(&[phone.device_id()]).await;
    // 插件已经在列表里：连上后取一次，卡片成功不再取。
    home.set_installed_plugins(vec![plugin()]);
    phone.connect(&home);
    phone.signed_in().await;
    let (core, room_id) = core_room(&phone, &home).await;
    let room = core.open_room(&room_id).await.expect("核心打开私聊");
    let mut timeline = Timeline::open(&phone, room_id.as_str()).await;
    wait_for(&phone, "连上后的第一次取列表", |view| {
        view.installed.is_some().then_some(())
    })
    .await;
    let baseline = home.installed_requests();

    let card = install_card("install");
    let posted = send(&room, content(&card)).await;
    until_card(&mut timeline, "install", "卡片出现", |view| {
        view.card.state == ConfirmationState::Pending
    })
    .await;
    followed_pending(&phone, "install").await;
    edit(
        &room,
        &posted,
        &with_state(&card, succeeded("装好了 Grafana")),
    )
    .await;
    until_card(&mut timeline, "install", "执行完了", |view| {
        matches!(view.card.state, ConfirmationState::Succeeded { .. })
    })
    .await;
    followed_settled(&phone, "install").await;
    assert_eq!(home.installed_requests(), baseline);

    // 卸载后（列表里没有了）：新一张成功卡片恰好补取一次，旧卡片不重试、不循环。
    home.set_installed_plugins(vec![]);
    phone.dispatch(Action::RefreshPlugins).unwrap();
    wait_for(&phone, "卸载后的列表", |view| {
        view.installed
            .as_ref()
            .is_some_and(|installed| installed.plugins.iter().all(|plugin| plugin.id != PLUGIN))
            .then_some(())
    })
    .await;
    let after_uninstall = home.installed_requests();
    let second = install_card("second");
    let posted = send(&room, content(&second)).await;
    until_card(&mut timeline, "second", "卡片出现", |view| {
        view.card.state == ConfirmationState::Pending
    })
    .await;
    followed_pending(&phone, "second").await;
    edit(&room, &posted, &with_state(&second, succeeded("又装好了"))).await;
    until_card(&mut timeline, "second", "执行完了", |view| {
        matches!(view.card.state, ConfirmationState::Succeeded { .. })
    })
    .await;
    followed_settled(&phone, "second").await;
    // 假核心回的是空列表，列表里不会有这个插件；等补取的请求落地。
    wait_for(&phone, "卸载后补取的请求", |_| {
        (home.installed_requests() == after_uninstall + 1).then_some(())
    })
    .await;
    assert_eq!(home.installed_requests(), after_uninstall + 1);
    // 同一张卡片再动一次：不再取。
    edit(
        &room,
        &posted,
        &with_state(&second, succeeded("还是装好了")),
    )
    .await;
    until_card(&mut timeline, "second", "重放后的结果", |view| {
        view.card.state
            == ConfirmationState::Succeeded {
                message: "还是装好了".to_owned(),
            }
    })
    .await;
    assert_eq!(home.installed_requests(), after_uninstall + 1);
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn startup_history_results_do_not_fetch() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let home = Home::start(&[phone.device_id()]).await;
    home.set_installed_plugins(vec![]);
    phone.connect(&home);
    phone.signed_in().await;
    let (core, room_id) = core_room(&phone, &home).await;
    let room = core.open_room(&room_id).await.expect("核心打开私聊");

    // 两张卡片直接以结果发出：历史里只有成功，没有本会话能看见的"没结束 → 成功"。
    let mut timeline = Timeline::open(&phone, room_id.as_str()).await;
    for id in ["install", "another"] {
        let done = with_state(&install_card(id), succeeded("装好了"));
        send(&room, content(&done)).await;
        until_card(&mut timeline, id, "结果出现", |view| {
            matches!(view.card.state, ConfirmationState::Succeeded { .. })
        })
        .await;
    }
    let before_restart = home.installed_requests();

    // 杀掉再启动：重启后的第一次取列表是重连触发的；跟随往回翻到历史里的成功卡片，不补取。
    drop(phone);
    let phone = Phone::open(dir.path());
    phone.signed_in().await;
    let deadline = Instant::now() + WAIT;
    while home.installed_requests() < before_restart + 1 {
        assert!(
            Instant::now() < deadline,
            "重启后没有取列表：{} 次",
            home.installed_requests()
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let mut timeline = Timeline::open(&phone, room_id.as_str()).await;
    until_card(&mut timeline, "another", "历史结果", |view| {
        matches!(view.card.state, ConfirmationState::Succeeded { .. })
    })
    .await;
    // 更早的那张在时间线订阅里不一定看得到（同步窗口有限），跟随自己回翻 50 条，两张都要认出来。
    wait_for(&phone, "跟随扫完历史", |_| {
        ["install", "another"]
            .iter()
            .all(|id| {
                phone
                    .chat()
                    .settled_confirmations
                    .iter()
                    .any(|card| card == id)
            })
            .then_some(())
    })
    .await;
    assert_eq!(home.installed_requests(), before_restart + 1);
}

/// #565 的不变量：卡片进了 settled_confirmations 时，“补不补取”已经定了。钩子把决定卡在报到和放行
/// 之间：settled 在放行前出现，就是修复前的行为——决定在另一个任务里，settled 谁先谁后看运气；
/// 修复后就地决定，卡着的时候 settled 发不出来。`biased` 加 settled 在前：两件事同时就绪（修复前
/// 几乎总是这样）时必须判失败，不能随机放过去。
#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn a_card_only_settles_after_the_refetch_decision_was_made() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let home = Home::start(&[phone.device_id()]).await;
    // 列表一开始就没有这个插件：连上后的第一次取列表就是基线。
    home.set_installed_plugins(vec![]);
    phone.connect(&home);
    phone.signed_in().await;
    let (core, room_id) = core_room(&phone, &home).await;
    let room = core.open_room(&room_id).await.expect("核心打开私聊");
    let mut timeline = Timeline::open(&phone, room_id.as_str()).await;
    wait_for(&phone, "连上后的第一次取列表", |view| {
        view.installed.is_some().then_some(())
    })
    .await;
    let baseline = home.installed_requests();

    // 钩子开着：卡片成功后决定先报到、等放行。
    phone.test_arm_card_success_hook();

    let card = install_card("install");
    let posted = send(&room, content(&card)).await;
    until_card(&mut timeline, "install", "卡片出现", |view| {
        view.card.state == ConfirmationState::Pending
    })
    .await;
    followed_pending(&phone, "install").await;
    edit(
        &room,
        &posted,
        &with_state(&card, succeeded("装好了 Grafana")),
    )
    .await;

    tokio::select! {
        biased;
        // settled 先到，而决定还卡在钩子上没做完：不变量破了。
        _ = settled_seen(&phone, "install") => {
            panic!("the card no longer needs an answer, but the refetch decision has not been made");
        }
        // 决定到了但卡着。修复前 settled 这时已经（或马上）会出现；修复后放行前它出不来。
        _ = phone.test_wait_card_success_arrived() => {
            if tokio::time::timeout(HELD, settled_seen(&phone, "install"))
                .await
                .is_ok()
            {
                panic!(
                    "the card no longer needs an answer, but the refetch decision has not been made"
                );
            }
            phone.test_release_card_success();
        }
        _ = tokio::time::sleep(WAIT) => {
            panic!(
                "neither the hook arrival (decision reached) nor settled (published by the follow) \
                 came within {WAIT:?}"
            );
        }
    }

    // 放行后 settled 才发出来，放行的那次决定也补取了。
    followed_settled(&phone, "install").await;
    wait_for(&phone, "放行后的补取", |_| {
        (home.installed_requests() == baseline + 1).then_some(())
    })
    .await;
}
