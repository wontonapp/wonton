//! 主人对单个 Bot 开关插件的接口（ADR-0066 第 5 节）：存下来、读得回，总线按它过滤（总线一侧见
//! `crate::mcp` 的 `bot_access_tests`）。

use hyper::{Method, StatusCode};
use wonton_protocol::{BotKeeper, bot_plugin_access_path};

use crate::gateway::family_tests::{bearer, send_as};
use crate::gateway::tests::{CORE, Harness, Reply};
use crate::matrix::BotAccount;
use crate::plugin::PluginId;
use crate::plugin::bots::{BotEntry, PluginBots};

const AGENT: &str = "app.agent";

/// `app.agent` 有一个叫 `default` 的 Bot；插件本身不用在运行，开关只看 Bot 记录。
fn seed_bot(h: &Harness) {
    let entry = BotEntry {
        account: BotAccount {
            user_id: "@app-agent:wonton.localhost".into(),
            home_room: "!home:wonton.localhost".into(),
        },
        device_id: None,
        keeper: BotKeeper::Household,
        bus_token: "bot-token".into(),
    };
    let bots = PluginBots {
        bots: [("default".to_owned(), entry)].into_iter().collect(),
    };
    h.manager
        .store()
        .bots()
        .save(&PluginId::parse(AGENT).unwrap(), &bots)
        .unwrap();
}

async fn send(h: &Harness, method: Method, bot: &str, target: &str, body: &str) -> Reply {
    send_as(
        &h.app,
        None,
        method,
        CORE,
        &bot_plugin_access_path(AGENT, bot, target),
        &[
            ("authorization", bearer().as_str()),
            ("content-type", "application/json"),
        ],
        body,
    )
    .await
}

fn is_off(h: &Harness) -> bool {
    h.manager
        .tool_policy()
        .is_bot_off(AGENT, "default", "app.whoami")
        .unwrap()
}

#[tokio::test]
async fn the_owner_switches_a_plugin_off_for_one_bot_and_reads_it_back() {
    let h = Harness::start(None).await;
    seed_bot(&h);

    let fresh = send(&h, Method::GET, "default", "app.whoami", "").await;
    assert_eq!(fresh.status, StatusCode::OK, "{}", fresh.text());
    assert_eq!(fresh.json()["plugin_id"], "app.whoami");
    assert_eq!(fresh.json()["source"], "global");
    assert_eq!(fresh.json()["enabled"], true, "没设过的 Bot 照旧能用");

    let off = send(
        &h,
        Method::PUT,
        "default",
        "app.whoami",
        "{\"enabled\":false}",
    )
    .await;
    assert_eq!(off.status, StatusCode::OK, "{}", off.text());
    assert_eq!(off.json()["enabled"], false);
    assert!(is_off(&h));
    let read = send(&h, Method::GET, "default", "app.whoami", "").await;
    assert_eq!(read.json()["enabled"], false);

    let on = send(
        &h,
        Method::PUT,
        "default",
        "app.whoami",
        "{\"enabled\":true}",
    )
    .await;
    assert_eq!(on.json()["enabled"], true);
    assert!(!is_off(&h));
}

#[tokio::test]
async fn unknown_bots_and_plugins_that_are_not_installed_are_told_apart_and_nothing_is_saved() {
    let h = Harness::start(None).await;
    seed_bot(&h);

    let no_bot = send(
        &h,
        Method::PUT,
        "other",
        "app.whoami",
        "{\"enabled\":false}",
    )
    .await;
    assert_eq!(no_bot.status, StatusCode::NOT_FOUND, "{}", no_bot.text());
    assert_eq!(no_bot.error_kind(), Some("not_found"));

    let missing = "app.missing";
    let no_target = send(&h, Method::PUT, "default", missing, "{\"enabled\":false}").await;
    assert_eq!(
        no_target.error_kind(),
        Some("not_installed"),
        "{}",
        no_target.text()
    );

    let policy = h.manager.tool_policy();
    assert!(!policy.is_bot_off(AGENT, "other", "app.whoami").unwrap());
    assert!(!policy.is_bot_off(AGENT, "default", missing).unwrap());
}
