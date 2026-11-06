use std::time::Duration;

use axum::http::Method;
use wonton_client::{Action, CoreError};

use super::rig::{Rig, WAIT, running};

#[tokio::test]
async fn bots_from_installed_plugins_keep_fallbacks_and_lose_uncertain_activity() {
    let rig = Rig::new().await;
    let mut bot = running("app.bot");
    bot.title = "Plugin name".into();
    bot.bot_user_id = Some("@bot:home".into());
    let mut empty = running("app.empty");
    empty.bot_user_id = Some(String::new());
    rig.core
        .set_lists(vec![vec![running("app.no-bot"), empty, bot.clone()]]);
    rig.core.hold(Method::GET, "/_wonton/api/bots/activity");
    rig.connected().await;
    let view = wait(&rig, |bots| bots.len() == 1).await;
    assert_eq!(view[0].name, "Plugin name");
    assert_eq!(view[0].avatar, None);
    assert_eq!(view[0].activity, None);
    rig.core.release(Method::GET, "/_wonton/api/bots/activity");
    assert!(matches!(
        rig.app.dispatch(Action::SetBotName {
            plugin_id: "app.bot".into(),
            name: "New name".into(),
        }),
        Err(CoreError::NotReady { .. })
    ));
    assert_eq!(
        rig.app.chat().bots[0].problem.as_ref().unwrap().reason,
        wonton_client::BotProfileFailure::NotReady
    );
    assert_eq!(rig.app.chat().bots[0].activity, None);
    rig.core.set_lists(vec![vec![]]);
    rig.app.dispatch(Action::RefreshPlugins).unwrap();
    wait(&rig, |bots| bots.is_empty()).await;
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
