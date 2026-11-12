use super::*;
use wonton_protocol::{BotActivityReport, BotWebPage, BotWebView, TabDisplay};

fn facts(name: Option<&str>, user: &str) -> BotActivityFacts {
    BotActivityFacts {
        plugin_id: "app.bot".into(),
        bot: name.map(str::to_owned),
        user_id: user.into(),
        running: true,
        model_requests: 0,
        tool_calls: 0,
        waiting: 0,
        profile_rev: 1,
    }
}

fn report(local: &mut Local, bots: Vec<BotActivityFacts>) {
    local.report = Some(BotActivityReport { rev: 1, bots });
}

fn ready(bot: &BotRef, url: &str) -> BotWebView {
    BotWebView::Ready {
        page: BotWebPage {
            bot: bot.clone(),
            url: url.into(),
            display: TabDisplay::default(),
            cross_origin_isolated: false,
        },
    }
}

#[test]
fn two_bots_keep_their_core_recorded_identity_profiles_activity_and_web() {
    let mut local = Local::default();
    let mut first = facts(Some("default"), "@not-the-bot-name:home");
    first.model_requests = 2;
    report(
        &mut local,
        vec![first, facts(Some("alice"), "@opaque-identity:home")],
    );
    let mut next = 0;
    let mut serial = || {
        next += 1;
        next
    };
    let (mut views, refresh) = reconcile(&mut local, &[], &Default::default(), &mut serial);
    assert_eq!(refresh.len(), 2);
    assert_eq!(
        views
            .iter()
            .map(|bot| (&*bot.bot.bot, &*bot.user_id))
            .collect::<Vec<_>>(),
        [
            ("default", "@not-the-bot-name:home"),
            ("alice", "@opaque-identity:home")
        ]
    );
    views[0].name = "Default profile".into();
    views[1].name = "Alice profile".into();
    views[0].web = Some(ready(&views[0].bot, "https://default.example/"));
    views[1].web = Some(ready(&views[1].bot, "https://alice.example/"));
    let alice = views[1].bot.clone();
    local
        .records
        .get_mut(&alice)
        .unwrap()
        .typing
        .insert("!alice:home".into(), true);
    let (views, refresh) = reconcile(&mut local, &views, &Default::default(), serial);
    assert!(refresh.is_empty());
    assert_eq!(views[0].name, "Default profile");
    assert_eq!(views[1].name, "Alice profile");
    assert_eq!(
        views[0].web,
        Some(ready(&views[0].bot, "https://default.example/"))
    );
    assert_eq!(views[1].web, Some(ready(&alice, "https://alice.example/")));
    assert_eq!(
        local.records[&views[0].bot]
            .facts
            .as_ref()
            .unwrap()
            .model_requests,
        2
    );
    assert_eq!(
        local.records[&alice].facts.as_ref().unwrap().model_requests,
        0
    );
    assert!(local.records[&alice].typing["!alice:home"]);
    assert!(local.records[&views[0].bot].typing.is_empty());
}

#[test]
fn missing_names_and_conflicting_identities_never_invent_a_target() {
    let mut local = Local::default();
    report(
        &mut local,
        vec![
            facts(None, "@app.bot.default:home"),
            facts(Some(""), "@alice:home"),
            facts(Some("duplicate"), "@one:home"),
            facts(Some("duplicate"), "@two:home"),
            facts(Some("real"), "@unrelated:home"),
        ],
    );
    let (views, _) = reconcile(&mut local, &[], &Default::default(), || 1);
    assert_eq!(views.len(), 1);
    assert_eq!(views[0].bot.bot, "real");
    assert_eq!(views[0].user_id, "@unrelated:home");
}

#[test]
fn removed_or_rebound_bot_cannot_keep_an_old_profile_or_web_page() {
    let mut local = Local::default();
    report(&mut local, vec![facts(Some("alice"), "@old:home")]);
    let (mut old, _) = reconcile(&mut local, &[], &Default::default(), || 1);
    old[0].web = Some(ready(&old[0].bot, "https://old.example/"));
    old[0].name = "Old account".into();
    report(&mut local, vec![facts(Some("alice"), "@new:home")]);
    let (new, refresh) = reconcile(&mut local, &old, &Default::default(), || 2);
    assert_eq!(new[0].user_id, "@new:home");
    assert_eq!(new[0].web, None);
    assert_ne!(new[0].name, "Old account");
    assert_eq!(refresh, [new[0].bot.clone()]);
    report(&mut local, vec![]);
    let (removed, _) = reconcile(&mut local, &new, &Default::default(), || 3);
    assert!(removed.is_empty());
    assert!(local.records.is_empty());
}
