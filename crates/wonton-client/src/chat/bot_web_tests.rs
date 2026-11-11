use super::*;
use std::sync::Mutex;
use std::sync::atomic::AtomicU64;
use tokio::sync::{Notify, watch};
use wonton_protocol::{
    BotActivityFacts, BotView, BotWebPage, ChatViewModel, HomeStatus, TabDisplay,
};

use crate::chat::bot_activity::Activity;
use crate::chat::bots::{Local, Record};
use crate::chat::layout::Layouts;

fn empty_view() -> ChatViewModel {
    ChatViewModel {
        chat_layout_default: Default::default(),
        account: wonton_protocol::ChatAccount::SignedOut,
        sync: wonton_protocol::ChatSync::Idle,
        encryption: wonton_protocol::EncryptionStatus {
            verification: wonton_protocol::DeviceVerification::Unknown,
            recovery: wonton_protocol::RecoveryStatus::Unknown,
            recovery_key: None,
            backup_reminder: None,
            can_verify_with_other_device: false,
            can_reset: false,
            recovery_next_step: wonton_protocol::RecoveryNextStep::None,
            verification_flow: None,
        },
        installing_bots: Vec::new(),
        rooms: Vec::new(),
        problem: None,
        push: wonton_protocol::PushStatus::NoEndpoint,
        pending_confirmations: Vec::new(),
        settled_confirmations: Vec::new(),
        background_notifications: Vec::new(),
        bots: Vec::new(),
    }
}

fn bot(name: &str) -> BotRef {
    BotRef {
        plugin_id: "app.bot".into(),
        bot: name.into(),
    }
}

fn ready(id: BotRef) -> BotWebView {
    BotWebView::Ready {
        page: BotWebPage {
            url: format!("https://{}.example/", id.bot),
            bot: id,
            display: TabDisplay::default(),
            cross_origin_isolated: false,
        },
    }
}

fn home() -> HomeViewModel {
    HomeViewModel {
        device_id: "phone".into(),
        server_id: Some("server".into()),
        status: HomeStatus::Connected,
        path: None,
        notice: None,
        details: Default::default(),
        proxy: None,
        web_url: None,
        pairing: None,
        invite: None,
        keep_connection: false,
    }
}

fn record() -> Record {
    Record {
        user_id: "@opaque:home".into(),
        title: "Bot".into(),
        facts: Some(BotActivityFacts {
            plugin_id: "app.bot".into(),
            bot: Some("alice".into()),
            user_id: "@opaque:home".into(),
            running: true,
            model_requests: 0,
            tool_calls: 0,
            waiting: 0,
            profile_rev: 1,
        }),
        profile_rev: Some(1),
        typing: Default::default(),
        activity: Activity::default(),
        refresh: 0,
        identity: 2,
    }
}

fn request() -> Request {
    Request {
        bot: bot("alice"),
        user: "@opaque:home".into(),
        identity: 2,
        generation: 3,
        epoch: 0,
        home: home(),
    }
}

fn profile(id: BotRef, user: &str) -> BotView {
    BotView {
        bot: id,
        user_id: user.into(),
        web: None,
        name: "Bot".into(),
        avatar: None,
        state_avatars: vec![],
        activity: None,
        saving: false,
        problem: None,
        chat: Default::default(),
    }
}

#[test]
fn metadata_never_authorizes_a_different_bot_or_an_unknown_state() {
    let alice = bot("alice");
    assert_eq!(
        exact(&alice, Some(ready(alice.clone()))),
        Some(ready(alice.clone()))
    );
    assert_eq!(exact(&alice, Some(ready(bot("default")))), None);
    let mut other_plugin = alice.clone();
    other_plugin.plugin_id = "app.other".into();
    assert_eq!(exact(&alice, Some(ready(other_plugin))), None);
    assert_eq!(exact(&alice, Some(BotWebView::Other)), None);
    assert_eq!(exact(&alice, None), None);
    let denied = BotWebView::Unavailable {
        reason: BotWebFailure::NotShared,
    };
    assert_eq!(exact(&alice, Some(denied.clone())), Some(denied));
}

#[test]
fn two_bots_publish_only_their_own_readiness_and_clear_errors_on_the_same_watch() {
    let request = request();
    let mut local = Local {
        web_generation: 3,
        ..Default::default()
    };
    local.records.insert(request.bot.clone(), record());
    let default = bot("default");
    let mut other = record();
    other.user_id = "@default:home".into();
    other.identity = 4;
    local.records.insert(default.clone(), other);
    let sender = watch::Sender::new(ChatViewModel {
        bots: vec![
            profile(default.clone(), "@default:home"),
            profile(request.bot.clone(), &request.user),
        ],
        ..empty_view()
    });
    let mut subscription = sender.subscribe();
    sender.send_if_modified(|view| apply(view, &local, &request, Some(ready(request.bot.clone()))));
    assert!(subscription.borrow_and_update().bots[0].web.is_none());
    assert_eq!(
        subscription.borrow().bots[1].web,
        Some(ready(request.bot.clone()))
    );
    let other_request = Request {
        bot: default.clone(),
        user: "@default:home".into(),
        identity: 4,
        generation: 3,
        epoch: 0,
        home: home(),
    };
    let stale = Request {
        bot: request.bot.clone(),
        user: request.user.clone(),
        identity: request.identity,
        generation: 2,
        epoch: request.epoch,
        home: home(),
    };
    sender.send_if_modified(|view| apply(view, &local, &stale, None));
    assert_eq!(
        subscription.borrow_and_update().bots[1].web,
        Some(ready(stale.bot.clone()))
    );
    sender
        .send_if_modified(|view| apply(view, &local, &other_request, Some(ready(default.clone()))));
    assert_eq!(
        subscription.borrow_and_update().bots[0].web,
        Some(ready(default))
    );
    for response in [Some(ready(bot("default"))), Some(BotWebView::Other), None] {
        sender.send_if_modified(|view| apply(view, &local, &request, response));
        assert!(subscription.borrow_and_update().bots[1].web.is_none());
    }
    local.clear_runtime();
    sender.send_modify(|view| view.bots.iter_mut().for_each(|bot| bot.web = None));
    assert!(!apply(
        &mut sender.borrow().clone(),
        &local,
        &request,
        Some(ready(request.bot.clone()))
    ));
    assert!(
        subscription
            .borrow_and_update()
            .bots
            .iter()
            .all(|bot| bot.web.is_none())
    );
}

#[test]
fn delayed_metadata_cannot_overwrite_a_newer_request_or_rebound_identity() {
    let mut local = Local {
        web_generation: 3,
        ..Default::default()
    };
    let request = request();
    local.records.insert(request.bot.clone(), record());
    assert!(current(&local, &request));
    local.web_generation += 1;
    assert!(!current(&local, &request));
    local.web_generation = 3;
    local.records.get_mut(&request.bot).unwrap().identity += 1;
    assert!(!current(&local, &request));
    local.records.get_mut(&request.bot).unwrap().identity = 2;
    local.records.get_mut(&request.bot).unwrap().user_id = "@new:home".into();
    assert!(!current(&local, &request));
    local.records.insert(request.bot.clone(), record());
    local.clear_runtime();
    assert!(!current(&local, &request));
    assert!(local.records[&request.bot].facts.is_none());
}

#[tokio::test]
async fn background_disconnect_and_session_change_clear_the_existing_chat_watch() {
    let dir = tempfile::tempdir().unwrap();
    let runtime = tokio::runtime::Handle::current();
    let home_view = watch::Sender::new(home());
    let (_, method) = watch::channel(wonton_protocol::PushMethod::default());
    let home = crate::home::HomeLink::new(
        runtime.clone(),
        crate::home::Storage::open(dir.path(), dir.path()).unwrap(),
        crate::home::Network::DirectOnly,
        Some(home_view.clone()),
        method,
    )
    .unwrap();
    let plugins = crate::plugins::PluginsLink::new(runtime.clone(), home.clone(), dir.path(), None);
    let bots = Arc::new(Bots {
        runtime,
        home,
        plugins,
        view: watch::Sender::new(empty_view()),
        session: watch::Sender::new(None),
        layouts: Layouts::new(),
        foreground: watch::Sender::new(true),
        changed: Notify::new(),
        epoch: AtomicU64::new(0),
        refresh_serial: AtomicU64::new(0),
        local: Mutex::default(),
    });
    let mut subscription = bots.view.subscribe();
    for transition in 0..3 {
        let request = request();
        lock(&bots.local)
            .records
            .insert(request.bot.clone(), record());
        lock(&bots.local).web_generation = request.generation;
        let mut view = profile(request.bot.clone(), &request.user);
        view.web = Some(ready(request.bot.clone()));
        bots.view.send_modify(|chat| chat.bots = vec![view]);
        if transition == 1 {
            home_view.send_modify(|view| view.status = HomeStatus::NotConfigured);
        }
        match transition {
            0 => bots.backgrounded(),
            1 => {
                bots.foreground.send_replace(true);
                let task = tokio::spawn(crate::chat::bot_poll::follow(
                    Arc::downgrade(&bots),
                    home_view.subscribe(),
                    bots.foreground.subscribe(),
                ));
                subscription.changed().await.unwrap();
                while subscription.borrow_and_update().bots[0].web.is_some() {
                    subscription.changed().await.unwrap();
                }
                task.abort();
            }
            _ => bots.set_session(None),
        }
        bots.apply_web(&request, Some(ready(request.bot.clone())));
        assert!(subscription.borrow_and_update().bots[0].web.is_none());
        assert!(!current(&lock(&bots.local), &request));
    }
    let mut default = bots.view.borrow().bots[0].clone();
    default.bot.bot = "default".into();
    default.user_id = "@default:home".into();
    bots.view.send_modify(|view| view.bots.push(default));
    assert!(bots.set_name("app.bot".into(), String::new()).is_err());
    assert!(bots.view.borrow().bots[0].problem.is_none());
    assert!(bots.view.borrow().bots[1].problem.is_some());
    bots.dismiss_problem("app.bot");
    assert!(bots.view.borrow().bots[1].problem.is_none());
    bots.view.send_modify(|view| view.bots.truncate(1));
    assert!(
        bots.set_name("app.bot".into(), "Valid name".into())
            .is_err()
    );
    assert!(bots.view.borrow().bots[0].problem.is_none());
}

#[test]
fn known_denial_is_preserved_but_transport_or_unknown_errors_have_no_page() {
    let error = |kind: &str| CoreApiError::Http {
        status: 403,
        kind: Some(kind.into()),
        message: String::new(),
        problem: None,
    };
    assert_eq!(
        failure(error("not_shared")),
        Some(BotWebView::Unavailable {
            reason: BotWebFailure::NotShared,
        })
    );
    assert_eq!(failure(error("future_reason")), None);
    assert_eq!(
        failure(CoreApiError::Protocol("missing metadata".into())),
        None
    );
}

#[test]
fn a_changed_account_session_server_or_connection_rejects_delayed_metadata() {
    let request = request();
    let mut current = home();
    assert!(requester_current(&request, &current, 0, true, true));
    assert!(!requester_current(&request, &current, 1, true, true));
    assert!(!requester_current(&request, &current, 0, false, true));
    assert!(!requester_current(&request, &current, 0, true, false));
    current.server_id = Some("another-server".into());
    assert!(!requester_current(&request, &current, 0, true, true));
    current = home();
    current.status = HomeStatus::NotConfigured;
    assert!(!requester_current(&request, &current, 0, true, true));
}
