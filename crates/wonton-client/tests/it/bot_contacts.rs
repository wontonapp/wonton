//! Bot 资料页的“联系人”（ADR-0045、ADR-0050）的行为测试：动作各打各的接口，结果和失败类别落进 ViewModel，
//! 有一行在等对方时自己跟上对方的处理。对面是本机假扮的核心管理接口。

use std::time::{Duration, Instant};

use axum::http::{Method, StatusCode};
use serde_json::{Value, json};
use wonton_client::{
    Action, BotContactFailure, BotContactsState, BotContactsViewModel, ContactError, ContactKind,
    ContactState,
};
use wonton_protocol::{
    BotRef, CONTACT_ERROR_HEADER, bot_contact_approve_path, bot_contact_path,
    bot_contact_reject_path, bot_contacts_path,
};

use crate::plugins::rig::{Canned, Headered, Rig, WAIT};

const PERSON: &str = "@sam:b.example";
const BOT: &str = "@hermes-bob:b.example";

fn alice() -> BotRef {
    BotRef {
        plugin_id: "ai.nous.hermes".into(),
        bot: "alice".into(),
    }
}

fn contact(user_id: &str, kind: &str, state: &str) -> Value {
    json!({
        "user_id": user_id,
        "display_name": null,
        "keeper_display_name": null,
        "kind": kind,
        "state": state,
        "remote": {"server_name": "b.example", "peer_status": "connected"},
    })
}

fn list(contacts: &[Value]) -> Canned {
    Canned(
        StatusCode::OK.as_u16(),
        json!({ "contacts": contacts }).to_string(),
    )
}

fn contact_error(status: u16, kind: &str) -> Headered {
    Headered(
        status,
        vec![(CONTACT_ERROR_HEADER.to_owned(), kind.to_owned())],
        json!({"error": {"kind": kind, "message": "核心写的说明"}}).to_string(),
    )
}

async fn wait_contacts(
    rig: &Rig,
    mut done: impl FnMut(&BotContactsViewModel) -> bool,
) -> BotContactsViewModel {
    let updates = rig.app.subscribe_bot_contacts();
    let waiting = async {
        loop {
            let view = rig.app.bot_contacts();
            if done(&view) {
                return view;
            }
            updates.next().await.expect("订阅提前结束");
        }
    };
    tokio::time::timeout(WAIT, waiting)
        .await
        .unwrap_or_else(|_| panic!("等待超时，当前：{:?}", rig.app.bot_contacts()))
}

fn ready(view: &BotContactsViewModel) -> bool {
    view.state == BotContactsState::Ready
}

#[tokio::test]
async fn refreshing_shows_the_bots_contacts() {
    let rig = Rig::new().await;
    rig.connected().await;
    let path = bot_contacts_path(&alice());
    rig.core.answer(
        Method::GET,
        &path,
        list(&[
            contact(BOT, "bot", "incoming"),
            contact(PERSON, "person", "active"),
        ]),
    );
    rig.app
        .dispatch(Action::BotContacts {
            action: wonton_protocol::BotContactAction::RefreshBotContacts { bot: alice() },
        })
        .unwrap();
    let view = wait_contacts(&rig, |view| ready(view) && !view.contacts.is_empty()).await;
    assert_eq!(view.bot, Some(alice()));
    assert_eq!(view.contacts.len(), 2);
    assert_eq!(view.contacts[0].state, ContactState::Incoming);
    assert_eq!(view.contacts[1].kind, ContactKind::Person);
    assert_eq!(
        view.contacts[0].remote.as_ref().unwrap().server_name,
        "b.example"
    );
}

/// 请一个人：请求体带用户 ID 和“人”；等他接受时自己隔几秒再取，他接受了就显示出来，不用壳再刷新。
#[tokio::test]
async fn a_pending_person_turns_active_without_another_refresh() {
    let rig = Rig::new().await;
    rig.connected().await;
    let path = bot_contacts_path(&alice());
    rig.core.answer(
        Method::POST,
        &path,
        list(&[contact(PERSON, "person", "pending")]),
    );
    rig.core.answer_always(
        Method::GET,
        &path,
        list(&[contact(PERSON, "person", "active")]),
    );
    rig.app
        .dispatch(Action::BotContacts {
            action: wonton_protocol::BotContactAction::RequestPersonContact {
                bot: alice(),
                user_id: PERSON.into(),
            },
        })
        .unwrap();
    wait_contacts(&rig, |view| {
        view.contacts.first().map(|c| c.state) == Some(ContactState::Pending)
    })
    .await;
    let sent = rig.core.seen(&Method::POST, &path);
    let body: Value = serde_json::from_str(&sent[0].body).unwrap();
    assert_eq!(body, json!({"user_id": PERSON, "kind": "person"}));

    wait_contacts(&rig, |view| {
        view.contacts.first().map(|c| c.state) == Some(ContactState::Active)
    })
    .await;
    // 没有在等的了就不再取。
    let polled = rig.core.seen(&Method::GET, &path).len();
    tokio::time::sleep(Duration::from_secs(7)).await;
    assert_eq!(rig.core.seen(&Method::GET, &path).len(), polled);
}

/// 对方拒绝了：刚发出请求的这段时间里取得勤，不用等满一轮慢的轮询（5 秒）就显示“对方拒绝了”。
#[tokio::test]
async fn a_rejection_by_the_other_side_shows_without_waiting_out_the_slow_poll() {
    let rig = Rig::new().await;
    rig.connected().await;
    let path = bot_contacts_path(&alice());
    rig.core
        .answer(Method::POST, &path, list(&[contact(BOT, "bot", "pending")]));
    rig.core
        .answer_always(Method::GET, &path, list(&[contact(BOT, "bot", "pending")]));
    rig.app
        .dispatch(Action::BotContacts {
            action: wonton_protocol::BotContactAction::RequestBotContact {
                bot: alice(),
                user_id: BOT.into(),
            },
        })
        .unwrap();
    wait_contacts(&rig, |view| {
        view.contacts.first().map(|c| c.state) == Some(ContactState::Pending)
    })
    .await;
    tokio::time::sleep(Duration::from_millis(1500)).await;

    rig.core
        .answer_always(Method::GET, &path, list(&[contact(BOT, "bot", "declined")]));
    let rejected = Instant::now();
    wait_contacts(&rig, |view| {
        view.contacts.first().map(|c| c.state) == Some(ContactState::Declined)
    })
    .await;
    assert!(
        rejected.elapsed() < Duration::from_secs(3),
        "对方拒绝后 {:?} 才显示",
        rejected.elapsed()
    );
}

/// 自己拒绝：拒绝接口回的列表当场落进 ViewModel，不等下一次取列表。取列表一直回“还在等你”，所以列表变空只能是
/// 拒绝自己的回应。
#[tokio::test]
async fn rejecting_shows_at_once_from_the_rejections_own_answer() {
    let rig = Rig::new().await;
    rig.connected().await;
    let path = bot_contacts_path(&alice());
    let reject = bot_contact_reject_path(&alice(), BOT);
    rig.core
        .answer_always(Method::GET, &path, list(&[contact(BOT, "bot", "incoming")]));
    rig.app
        .dispatch(Action::BotContacts {
            action: wonton_protocol::BotContactAction::RefreshBotContacts { bot: alice() },
        })
        .unwrap();
    wait_contacts(&rig, |view| ready(view) && view.contacts.len() == 1).await;
    rig.core.answer(Method::POST, &reject, list(&[]));
    rig.app
        .dispatch(Action::BotContacts {
            action: wonton_protocol::BotContactAction::RejectBotContact {
                bot: alice(),
                user_id: BOT.into(),
            },
        })
        .unwrap();
    wait_contacts(&rig, |view| ready(view) && view.contacts.is_empty()).await;
    assert_eq!(rig.core.seen(&Method::POST, &reject).len(), 1);
}

#[tokio::test]
async fn approving_rejecting_and_removing_hit_the_contacts_own_paths() {
    let rig = Rig::new().await;
    rig.connected().await;
    let bot = alice();
    let approve = bot_contact_approve_path(&bot, BOT);
    let reject = bot_contact_reject_path(&bot, PERSON);
    let remove = bot_contact_path(&bot, BOT);
    rig.core.answer(
        Method::POST,
        &approve,
        list(&[contact(BOT, "bot", "active")]),
    );
    rig.core.answer(
        Method::POST,
        &reject,
        list(&[contact(BOT, "bot", "active")]),
    );
    rig.core.answer(Method::DELETE, &remove, list(&[]));

    rig.app
        .dispatch(Action::BotContacts {
            action: wonton_protocol::BotContactAction::ApproveBotContact {
                bot: bot.clone(),
                user_id: BOT.into(),
            },
        })
        .unwrap();
    wait_contacts(&rig, |view| {
        ready(view) && view.contacts.first().map(|c| c.state) == Some(ContactState::Active)
    })
    .await;
    rig.app
        .dispatch(Action::BotContacts {
            action: wonton_protocol::BotContactAction::RejectBotContact {
                bot: bot.clone(),
                user_id: PERSON.into(),
            },
        })
        .unwrap();
    rig.app
        .dispatch(Action::BotContacts {
            action: wonton_protocol::BotContactAction::RemoveBotContact {
                bot: bot.clone(),
                user_id: BOT.into(),
            },
        })
        .unwrap();
    wait_contacts(&rig, |view| ready(view) && view.contacts.is_empty()).await;
    assert_eq!(rig.core.seen(&Method::POST, &approve).len(), 1);
    assert_eq!(rig.core.seen(&Method::POST, &reject).len(), 1);
    assert_eq!(rig.core.seen(&Method::DELETE, &remove).len(), 1);
}

/// 取列表失败照类别报：核心的错误读响应头，说明用核心写的那句；本地代理自己回的（没有头）按状态码分。
#[tokio::test]
async fn list_failures_keep_their_category() {
    let rig = Rig::new().await;
    rig.connected().await;
    let path = bot_contacts_path(&alice());
    rig.core
        .answer_headered(Method::GET, &path, contact_error(404, "bot_not_found"));
    rig.app
        .dispatch(Action::BotContacts {
            action: wonton_protocol::BotContactAction::RefreshBotContacts { bot: alice() },
        })
        .unwrap();
    let view = wait_contacts(&rig, |view| {
        matches!(view.state, BotContactsState::Failed { .. })
    })
    .await;
    assert_eq!(
        view.state,
        BotContactsState::Failed {
            error: ContactError::BotNotFound,
            message: "核心写的说明".into(),
        }
    );

    rig.core.answer(
        Method::GET,
        &path,
        Canned(StatusCode::FORBIDDEN.as_u16(), "device revoked".into()),
    );
    rig.app
        .dispatch(Action::BotContacts {
            action: wonton_protocol::BotContactAction::RefreshBotContacts { bot: alice() },
        })
        .unwrap();
    let view = wait_contacts(&rig, |view| {
        matches!(
            view.state,
            BotContactsState::Failed {
                error: ContactError::Unauthenticated,
                ..
            }
        )
    })
    .await;
    assert!(view.contacts.is_empty());
}

/// 发起、同意之类的动作失败不换掉列表：原因带着类别单独放，列表照旧；随后重取一次跟上别处的处理，
/// 壳的下一个动作清掉这次的失败。
#[tokio::test]
async fn an_action_failure_keeps_the_list_and_reports_on_its_own() {
    let rig = Rig::new().await;
    rig.connected().await;
    let path = bot_contacts_path(&alice());
    rig.core
        .answer_always(Method::GET, &path, list(&[contact(BOT, "bot", "active")]));
    rig.app
        .dispatch(Action::BotContacts {
            action: wonton_protocol::BotContactAction::RefreshBotContacts { bot: alice() },
        })
        .unwrap();
    wait_contacts(&rig, |view| ready(view) && view.contacts.len() == 1).await;
    let listed = rig.core.seen(&Method::GET, &path).len();

    rig.core
        .answer_headered(Method::POST, &path, contact_error(400, "invalid_user_id"));
    rig.app
        .dispatch(Action::BotContacts {
            action: wonton_protocol::BotContactAction::RequestBotContact {
                bot: alice(),
                user_id: "sam".into(),
            },
        })
        .unwrap();
    let view = wait_contacts(&rig, |view| view.action_failure.is_some()).await;
    assert_eq!(
        view.action_failure,
        Some(BotContactFailure {
            error: ContactError::InvalidUserId,
            message: "核心写的说明".into(),
        })
    );
    assert_eq!(view.state, BotContactsState::Ready);
    assert_eq!(view.contacts.len(), 1);

    tokio::time::timeout(WAIT, async {
        while rig.core.seen(&Method::GET, &path).len() == listed {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("动作失败后没有重取列表");
    let view = rig.app.bot_contacts();
    assert!(view.action_failure.is_some(), "后台重取不清掉动作的失败");
    assert_eq!(view.contacts.len(), 1);

    rig.app
        .dispatch(Action::BotContacts {
            action: wonton_protocol::BotContactAction::RefreshBotContacts { bot: alice() },
        })
        .unwrap();
    assert_eq!(rig.app.bot_contacts().action_failure, None);
}

/// 刚切到一个还没取过列表的 Bot 就做动作、又失败了：没有列表可留，重取一次补上。
#[tokio::test]
async fn an_action_failure_on_a_fresh_bot_fetches_its_list() {
    let rig = Rig::new().await;
    rig.connected().await;
    let path = bot_contacts_path(&alice());
    let approve = bot_contact_approve_path(&alice(), BOT);
    rig.core.answer_always(
        Method::GET,
        &path,
        list(&[contact(PERSON, "person", "active")]),
    );
    rig.core.answer_headered(
        Method::POST,
        &approve,
        contact_error(404, "contact_not_found"),
    );
    rig.app
        .dispatch(Action::BotContacts {
            action: wonton_protocol::BotContactAction::ApproveBotContact {
                bot: alice(),
                user_id: BOT.into(),
            },
        })
        .unwrap();
    let view = wait_contacts(&rig, |view| ready(view) && view.contacts.len() == 1).await;
    assert_eq!(
        view.action_failure.map(|failure| failure.error),
        Some(ContactError::ContactNotFound)
    );
    assert_eq!(view.contacts[0].user_id, PERSON);
}

#[tokio::test]
async fn without_a_home_connection_the_transport_is_reported() {
    let rig = Rig::new().await;
    rig.app
        .dispatch(Action::BotContacts {
            action: wonton_protocol::BotContactAction::RefreshBotContacts { bot: alice() },
        })
        .unwrap();
    let view = wait_contacts(&rig, |view| {
        matches!(view.state, BotContactsState::Failed { .. })
    })
    .await;
    assert!(matches!(
        view.state,
        BotContactsState::Failed {
            error: ContactError::TransportUnavailable,
            ..
        }
    ));
    assert!(
        rig.core
            .all_seen()
            .iter()
            .all(|seen| !seen.path.contains("/contacts"))
    );
}
