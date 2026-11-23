use std::time::Duration;

use axum::http::Method;
use wonton_client::{Action, BotActivity, BotProfileFailure, CoreError};
use wonton_protocol::BotChatAvailability;
use wonton_protocol::{BotActivityFacts, BotActivityReport, BotRef};

use super::rig::{Rig, WAIT, running};

const ACTIVITY: &str = "/_wonton/api/bots/activity";

#[tokio::test]
async fn only_recorded_bots_appear_and_failed_activity_clears_their_capabilities() {
    let rig = Rig::new().await;
    let mut bot = running("app.bot");
    bot.title = "Plugin name".into();
    bot.bot_user_id = Some("@installed-default:home".into());
    bot.default_bot_visibility = Some(wonton_protocol::DefaultBotVisibility::Visible);
    let mut empty = running("app.empty");
    empty.bot_user_id = Some(String::new());
    empty.default_bot_visibility = Some(wonton_protocol::DefaultBotVisibility::Visible);
    rig.core
        .set_lists(vec![vec![running("app.no-bot"), empty, bot]]);
    rig.core.hold(Method::GET, ACTIVITY);
    rig.core.set_bot_activity(BotActivityReport {
        rev: 1,
        bots: vec![
            facts(Some("default"), "@authoritative:home"),
            facts(None, "@looks-like-alice:home"),
        ],
    });
    rig.connected().await;
    rig.wait_for(|view| {
        view.installed
            .as_ref()
            .is_some_and(|installed| installed.plugins.len() == 3)
    })
    .await;
    wait_request(&rig).await;
    // 已装插件的默认 Bot 直接按安装记录出现在列表里（活动报告还没到，先不显示状态）。
    let installed_view = wait(&rig, |bots| bots.len() == 1).await;
    assert_eq!(installed_view[0].user_id, "@installed-default:home");
    assert_eq!(installed_view[0].activity, None);
    rig.core.release(Method::GET, ACTIVITY);
    let view = wait(&rig, |bots| {
        bots.len() == 1 && bots[0].activity == Some(BotActivity::Idle)
    })
    .await;
    assert_eq!(
        view[0].bot,
        BotRef {
            plugin_id: "app.bot".into(),
            bot: "default".into()
        }
    );
    assert_eq!(view[0].user_id, "@authoritative:home");
    assert_eq!(view[0].name, "Plugin name");
    assert_eq!(view[0].avatar, None);
    assert_eq!(
        view[0].web, None,
        "An activity report is not web authorization"
    );
    assert!(matches!(
        rig.app.dispatch(Action::Bot {
            action: wonton_protocol::BotAction::SetBotName {
                plugin_id: "app.bot".into(),
                name: "New name".into(),
            }
        }),
        Err(CoreError::NotReady { .. })
    ));
    assert_eq!(
        rig.app.chat().bots[0].problem.as_ref().unwrap().reason,
        BotProfileFailure::NotReady
    );
    rig.core.fail_bot_activity();
    let view = wait(&rig, |bots| {
        bots.len() == 1 && bots[0].activity.is_none() && bots[0].web.is_none()
    })
    .await;
    assert_eq!(view[0].user_id, "@authoritative:home");
    // Plugin refresh cannot recreate a guessed default from installed metadata.
    rig.core.set_lists(vec![vec![]]);
    rig.app
        .dispatch(Action::Plugins {
            action: wonton_protocol::PluginAction::RefreshPlugins,
        })
        .unwrap();
    wait(&rig, |bots| bots.is_empty()).await;
}

#[tokio::test]
async fn plugin_refresh_does_not_switch_between_bots_of_the_same_plugin() {
    let rig = Rig::new().await;
    let mut plugin = running("app.bot");
    plugin.bot_user_id = Some("@default:home".into());
    plugin.default_bot_visibility = Some(wonton_protocol::DefaultBotVisibility::Visible);
    rig.core.set_lists(vec![vec![plugin.clone()]]);
    let mut alice = facts(Some("alice"), "@alice:home");
    alice.model_requests = 1;
    rig.core.set_bot_activity(BotActivityReport {
        rev: 1,
        bots: vec![alice, facts(Some("default"), "@default:home")],
    });
    // 列表落地时 Bot 列表按安装记录重建，只剩默认 Bot；报告先到、列表后到时，alice 要等下一份报告
    // （长轮询最长 25 秒）才回来。先等列表落地再放行报告，两者的先后不靠调度运气。
    rig.core.hold(Method::GET, ACTIVITY);
    rig.connected().await;
    wait(&rig, |bots| bots.len() == 1 && bots[0].bot.bot == "default").await;
    rig.core.release(Method::GET, ACTIVITY);
    let before = wait(&rig, |bots| {
        bots.len() == 2
            && bots[0].bot.bot == "alice"
            && bots[0].activity == Some(BotActivity::Working)
            && bots[1].bot.bot == "default"
            && bots[1].activity == Some(BotActivity::Idle)
    })
    .await;

    rig.core.hold(Method::GET, ACTIVITY);
    plugin.title = "Updated".into();
    rig.core.set_lists(vec![vec![plugin]]);
    rig.app
        .dispatch(Action::Plugins {
            action: wonton_protocol::PluginAction::RefreshPlugins,
        })
        .unwrap();
    rig.wait_for(|view| {
        view.installed.as_ref().is_some_and(|installed| {
            installed
                .plugins
                .first()
                .is_some_and(|plugin| plugin.title == "Updated")
        })
    })
    .await;
    // 插件重建只按安装记录做默认 Bot；报告里还有的 alice 按保留规则留在列表里，默认 Bot 不因此被换成 alice。
    let bots = wait(&rig, |bots| {
        bots.len() == 2 && bots.iter().any(|bot| bot.bot.bot == "default")
    })
    .await;
    let updated = bots
        .iter()
        .find(|bot| bot.bot.bot == "default")
        .expect("默认 Bot 还在");
    assert_eq!(updated.user_id, "@default:home");
    assert_eq!(updated.name, before[1].name);
    assert_eq!(updated.avatar, before[1].avatar);
    assert_eq!(updated.state_avatars, before[1].state_avatars);
    assert_eq!(updated.activity, Some(BotActivity::Idle));
    assert!(
        bots.iter().any(|bot| bot.bot.bot == "alice"),
        "报告里的 alice 保留到下一份报告"
    );
}

fn facts(bot: Option<&str>, user_id: &str) -> BotActivityFacts {
    BotActivityFacts {
        plugin_id: "app.bot".into(),
        bot: bot.map(str::to_owned),
        user_id: user_id.into(),
        chat: BotChatAvailability::Available,
        running: true,
        model_requests: 0,
        tool_calls: 0,
        waiting: 0,
        profile_rev: 0,
    }
}

async fn wait_request(rig: &Rig) {
    let deadline = tokio::time::Instant::now() + WAIT;
    while !rig
        .core
        .all_seen()
        .iter()
        .any(|request| request.method == Method::GET && request.path.starts_with(ACTIVITY))
    {
        assert!(
            tokio::time::Instant::now() < deadline,
            "Activity request was not received"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

async fn wait(
    rig: &Rig,
    check: impl Fn(&[wonton_client::BotView]) -> bool,
) -> Vec<wonton_client::BotView> {
    let deadline = tokio::time::Instant::now() + WAIT;
    loop {
        let bots = rig.app.chat().bots;
        if check(&bots) {
            return bots;
        }
        assert!(tokio::time::Instant::now() < deadline, "{bots:?}");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}
