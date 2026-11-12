use std::time::Duration;

use axum::http::Method;
use wonton_client::{Action, BotActivity, BotProfileFailure, CoreError};
use wonton_protocol::{BotActivityFacts, BotActivityReport, BotRef};

use super::rig::{Rig, WAIT, running};

const ACTIVITY: &str = "/_wonton/api/bots/activity";

#[tokio::test]
async fn only_recorded_bots_appear_and_failed_activity_clears_their_capabilities() {
    let rig = Rig::new().await;
    let mut bot = running("app.bot");
    bot.title = "Plugin name".into();
    bot.bot_user_id = Some("@installed-default:home".into());
    let mut empty = running("app.empty");
    empty.bot_user_id = Some(String::new());
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
    assert!(
        rig.app.chat().bots.is_empty(),
        "Installed default IDs do not establish BotRef names"
    );
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
        rig.app.dispatch(Action::SetBotName {
            plugin_id: "app.bot".into(),
            name: "New name".into(),
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
    rig.app.dispatch(Action::RefreshPlugins).unwrap();
    wait(&rig, |bots| bots.is_empty()).await;
}

fn facts(bot: Option<&str>, user_id: &str) -> BotActivityFacts {
    BotActivityFacts {
        plugin_id: "app.bot".into(),
        bot: bot.map(str::to_owned),
        user_id: user_id.into(),
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
