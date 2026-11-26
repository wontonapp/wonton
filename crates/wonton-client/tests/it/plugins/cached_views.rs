//! 断网和冷启动时看到的旧数据：上次取到的列表照常显示，只在当前成立的东西（Bot 身份）要等列表重取。

use std::time::Duration;

use axum::http::Method;
use wonton_client::HomeStatus;

use super::rig::{Rig, WAIT, running};

const PLUGINS: &str = "/_wonton/api/plugins";

/// 等对话里的 Bot 列表跟上插件视图；联系人只读 Bot 列表，不等会撞上两条订阅的先后。
async fn wait_bot(rig: &Rig, user: &str) {
    let deadline = tokio::time::Instant::now() + WAIT;
    loop {
        if rig.app.chat().bots.iter().any(|bot| bot.user_id == user) {
            return;
        }
        assert!(tokio::time::Instant::now() < deadline, "{user} 没出现");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

/// 冷启动没网也能显示主屏：一打开就有上次取到的列表，不等隧道。
#[tokio::test]
async fn the_installed_list_is_there_as_soon_as_the_app_opens() {
    let mut rig = Rig::new().await;
    rig.core.set_lists(vec![vec![running("app.notes")]]);
    rig.connected().await;
    rig.wait_for(|view| view.installed.is_some()).await;
    // 列表是取到之后才放进 ViewModel 的，这时已经存下了。

    rig.reopen();

    let installed = rig.app.plugins().installed.expect("没有上次的列表");
    assert_eq!(installed.plugins[0].id, "app.notes");
}

#[tokio::test]
async fn cached_bot_identity_is_not_a_current_chat_candidate() {
    const OLD_BOT: &str = "@old-bot:home";
    const CURRENT_BOT: &str = "@current-bot:home";
    let mut rig = Rig::new().await;
    let mut plugin = running("app.bot");
    plugin.bot_user_id = Some(OLD_BOT.to_owned());
    plugin.default_bot_visibility = Some(wonton_protocol::DefaultBotVisibility::Visible);
    rig.core.set_lists(vec![vec![plugin]]);
    rig.connected().await;
    rig.wait_for(|view| {
        view.installed
            .as_ref()
            .is_some_and(|installed| installed.plugins[0].bot_user_id.as_deref() == Some(OLD_BOT))
    })
    .await;

    let cache = rig.data_dir().join("installed-plugins.json");
    let mut cached: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&cache).unwrap()).unwrap();
    cached["installed"]["plugins"][0]["bot_user_id"] = serde_json::json!(OLD_BOT);
    std::fs::write(cache, serde_json::to_vec(&cached).unwrap()).unwrap();
    rig.core.hold(Method::GET, "/_wonton/api/plugins");
    rig.reopen();
    let installed = rig.app.plugins().installed.expect("cached plugin list");
    assert_eq!(installed.plugins[0].id, "app.bot");
    assert_eq!(installed.plugins[0].bot_user_id, None);
    assert!(
        rig.app
            .chat_contacts(String::new())
            .iter()
            .all(|contact| contact.user_id != OLD_BOT)
    );

    let mut current = running("app.bot");
    current.bot_user_id = Some(CURRENT_BOT.to_owned());
    current.default_bot_visibility = Some(wonton_protocol::DefaultBotVisibility::Visible);
    rig.core.set_lists(vec![vec![current]]);
    rig.core.release(Method::GET, "/_wonton/api/plugins");
    rig.connected().await;
    rig.wait_for(|view| {
        view.installed.as_ref().is_some_and(|installed| {
            installed.plugins[0].bot_user_id.as_deref() == Some(CURRENT_BOT)
        })
    })
    .await;
    wait_bot(&rig, CURRENT_BOT).await;
    assert!(
        rig.app
            .chat_contacts(String::new())
            .iter()
            .any(|contact| contact.user_id == CURRENT_BOT)
    );
}

#[tokio::test]
async fn unverified_bot_identity_is_not_a_chat_candidate() {
    let rig = Rig::new().await;
    let mut unverified = running("app.unverified");
    unverified.bot_user_id = Some("@private:home".into());
    let mut verified = running("app.verified");
    verified.bot_user_id = Some("@mine:home".into());
    verified.default_bot_visibility = Some(wonton_protocol::DefaultBotVisibility::Visible);
    rig.core.set_lists(vec![vec![unverified, verified]]);
    rig.connected().await;
    let view = rig
        .wait_for(|view| {
            view.installed
                .as_ref()
                .is_some_and(|list| list.plugins.len() == 2)
        })
        .await;
    let installed = view.installed.unwrap();
    assert_eq!(installed.plugins[0].bot_user_id, None);
    assert_eq!(
        installed.plugins[1].bot_user_id.as_deref(),
        Some("@mine:home")
    );
    wait_bot(&rig, "@mine:home").await;
    let contacts = rig.app.chat_contacts(String::new());
    assert!(
        contacts
            .iter()
            .all(|contact| contact.user_id != "@private:home")
    );
    assert!(
        contacts
            .iter()
            .any(|contact| contact.user_id == "@mine:home"),
        "bots: {:?}, contacts: {contacts:?}",
        rig.app.chat().bots
    );
}

#[tokio::test]
async fn malformed_default_identities_are_not_offered_but_absent_bots_get_their_id() {
    use wonton_protocol::DefaultBotVisibility;

    let rig = Rig::new().await;
    let mut malformed = running("app.malformed");
    malformed.bot_user_id = Some("not-a-matrix-id".into());
    malformed.default_bot_visibility = Some(DefaultBotVisibility::Visible);
    let mut contradictory = running("app.contradictory");
    contradictory.bot_user_id = Some("@secret:home".into());
    contradictory.default_bot_visibility = Some(DefaultBotVisibility::Absent);
    let mut visible_without_id = running("app.missing");
    visible_without_id.default_bot_visibility = Some(DefaultBotVisibility::Visible);
    let mut verified = running("app.verified");
    verified.bot_user_id = Some("@mine:home".into());
    verified.default_bot_visibility = Some(DefaultBotVisibility::Visible);
    rig.core.set_lists(vec![vec![
        malformed,
        contradictory,
        visible_without_id,
        verified,
    ]]);
    rig.connected().await;

    let view = rig
        .wait_for(|view| {
            view.installed
                .as_ref()
                .is_some_and(|list| list.plugins.len() == 4)
        })
        .await;
    let installed = view.installed.unwrap();
    let find = |id: &str| {
        installed
            .plugins
            .iter()
            .find(|plugin| plugin.id == id)
            .unwrap()
    };
    for id in ["app.malformed", "app.missing"] {
        assert_eq!(find(id).bot_user_id, None, "{id}");
        assert_eq!(
            find(id).default_bot_visibility,
            Some(DefaultBotVisibility::Unknown),
            "{id}"
        );
    }
    // 不可见但身份合法：留着身份，插件页点得开私聊。
    assert_eq!(
        find("app.contradictory").bot_user_id.as_deref(),
        Some("@secret:home")
    );
    assert_eq!(
        find("app.contradictory").default_bot_visibility,
        Some(DefaultBotVisibility::Absent)
    );
    wait_bot(&rig, "@secret:home").await;
    let contacts = rig.app.chat_contacts(String::new());
    assert!(
        contacts
            .iter()
            .any(|contact| contact.user_id == "@secret:home")
    );
    assert!(
        contacts
            .iter()
            .any(|contact| contact.user_id == "@mine:home")
    );
}

#[tokio::test]
async fn disconnect_keeps_last_known_bot_sendable_until_list_is_refetched() {
    let rig = Rig::new().await;
    let mut plugin = running("app.bot");
    plugin.bot_user_id = Some("@mine:home".into());
    plugin.default_bot_visibility = Some(wonton_protocol::DefaultBotVisibility::Visible);
    rig.core.set_lists(vec![vec![plugin]]);
    rig.connected().await;
    rig.wait_for(|view| {
        view.installed
            .as_ref()
            .is_some_and(|list| list.plugins[0].bot_user_id.as_deref() == Some("@mine:home"))
    })
    .await;
    wait_bot(&rig, "@mine:home").await;

    // 隧道每秒按真实连接重新发布状态，单次设置会被盖回“已连接”，盖得早时断开根本没被观察到。
    // 断言做完前一直设着“正在连接”；其间被盖回时触发的取列表按住不回，不让它把身份写回来。
    rig.core.hold(Method::GET, PLUGINS);
    let offline_guard = {
        let app = rig.app.clone();
        tokio::spawn(async move {
            loop {
                app.set_home_status_for_test(HomeStatus::Connecting);
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
    };
    let offline = rig
        .wait_for(|view| {
            view.installed.as_ref().is_some_and(|list| {
                list.plugins[0].default_bot_visibility
                    == Some(wonton_protocol::DefaultBotVisibility::Unknown)
            })
        })
        .await;
    assert_eq!(offline.installed.unwrap().plugins[0].bot_user_id, None);
    // 插件列表先藏掉身份，但对话里的 Bot 还是资格未知、照常能发，候选里也还在。
    assert!(
        rig.app
            .chat_contacts(String::new())
            .iter()
            .any(|contact| contact.user_id == "@mine:home")
    );
    offline_guard.abort();
    rig.core.release(Method::GET, PLUGINS);

    rig.app.set_home_status_for_test(HomeStatus::Connected);
    rig.wait_for(|view| {
        view.installed
            .as_ref()
            .is_some_and(|list| list.plugins[0].bot_user_id.as_deref() == Some("@mine:home"))
    })
    .await;
}
