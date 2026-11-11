use serde_json::{Value, json};
use wonton_protocol::MAIL_FIELD;

use super::delivery::Delivery;
use super::test_support::{Mailpit, PLUGIN, home, received};

fn owner() -> Value {
    json!([{"Address":"owner@wonton.internal"}])
}

#[tokio::test]
async fn recipient_union_reaches_each_current_persons_own_private_room_once() {
    let home = home().await;
    let mailpit = Mailpit::start().await;
    mailpit.add("union", owner(), received("172.30.1.5"));
    {
        let mut state = mailpit.state.lock().unwrap();
        let message = state.messages.get_mut("union").unwrap();
        message["To"] =
            json!([{"Address":"alice@wonton.internal"},{"Address":"outside@example.com"}]);
        message["Cc"] =
            json!([{"Address":"alice@wonton.internal"},{"Address":"missing@wonton.internal"}]);
        // Mailpit adds an envelope-only owner here. The header-only Alice is deliberately accepted too.
        message["Bcc"] =
            json!([{"Address":"owner@wonton.internal"},{"Address":"alice@wonton.internal"}]);
    }
    let mut delivery = Delivery::new(mailpit.api(), home.gateway.clone(), home.dir.path()).unwrap();
    delivery.deliver("union").await.unwrap();
    let matrix = home.matrix.state();
    assert_eq!(matrix.sent.len(), 2);
    let mut recipients = Vec::new();
    for (room, content) in &matrix.sent {
        assert_eq!(content["msgtype"], "m.text");
        assert_eq!(content[MAIL_FIELD]["plugin_id"], PLUGIN);
        assert_eq!(content[MAIL_FIELD]["from"], "arbitrary@example.com");
        let people: Vec<_> = matrix.rooms[room]
            .members
            .iter()
            .filter(|user| user.as_str() != "@wonton-core:test")
            .cloned()
            .collect();
        assert_eq!(people.len(), 1);
        recipients.extend(people);
    }
    recipients.sort();
    assert_eq!(recipients, ["@alice:test", "@owner:test"]);
    assert_eq!(mailpit.state.lock().unwrap().deleted, ["union"]);
}

#[tokio::test]
async fn an_untrusted_top_received_never_borrows_a_lower_plugins_identity() {
    let home = home().await;
    let mailpit = Mailpit::start().await;
    let mut headers = received("203.0.113.10");
    headers["Received"]
        .as_array_mut()
        .unwrap()
        .push(received("172.30.1.5")["Received"][0].clone());
    mailpit.add("forged", owner(), headers);
    let mut delivery = Delivery::new(mailpit.api(), home.gateway.clone(), home.dir.path()).unwrap();
    delivery.deliver("forged").await.unwrap();
    let matrix = home.matrix.state();
    assert_eq!(matrix.sent.len(), 1);
    assert_eq!(matrix.sent[0].1["msgtype"], "m.notice");
    assert!(matrix.sent[0].1.get(MAIL_FIELD).is_none());
    assert_eq!(mailpit.state.lock().unwrap().deleted, ["forged"]);
}

#[tokio::test]
async fn the_private_chat_names_the_network_sender_not_the_from_header() {
    let home = home().await;
    let mailpit = Mailpit::start().await;
    home.gateway.plugins.register_source_for_test(
        "app.test.other",
        "172.30.2.0/24",
        vec![crate::plugin::Permission::Notify],
    );
    // Both mails travel from the first plugin's network; only the From header names someone else.
    for (id, from, subject) in [
        (
            "other-from",
            "app.test.other@plugins.wonton.internal",
            "other",
        ),
        ("person-from", "alice@wonton.internal", "person"),
    ] {
        mailpit.add(id, owner(), received("172.30.1.5"));
        let mut state = mailpit.state.lock().unwrap();
        let message = state.messages.get_mut(id).unwrap();
        message["From"] = json!({"Address": from});
        message["Subject"] = json!(subject);
    }
    let mut delivery = Delivery::new(mailpit.api(), home.gateway.clone(), home.dir.path()).unwrap();
    for id in ["other-from", "person-from"] {
        delivery.deliver(id).await.unwrap();
    }
    let matrix = home.matrix.state();
    assert_eq!(matrix.sent.len(), 2);
    for (index, subject) in ["other", "person"].into_iter().enumerate() {
        let content = &matrix.sent[index].1;
        assert_eq!(content[MAIL_FIELD]["plugin_id"], PLUGIN);
        assert_ne!(content[MAIL_FIELD]["plugin_id"], "app.test.other");
        assert_eq!(content[MAIL_FIELD]["plugin_name"], "Mail app");
        let body = content["body"].as_str().unwrap();
        assert!(
            body.starts_with(&format!("Mail app：{subject}\n")),
            "{body}"
        );
    }
}

#[tokio::test]
async fn matrix_outage_keeps_the_mail_then_recovers_without_recounting_admission() {
    let home = home().await;
    let mailpit = Mailpit::start().await;
    mailpit.add("recover", owner(), received("172.30.1.5"));
    let plugin = crate::plugin::PluginId::parse(PLUGIN).unwrap();
    for _ in 0..9 {
        assert!(home.gateway.notify_limiter.admit(&plugin).is_ok());
    }
    home.matrix.state().refuse_rooms = true;
    let mut delivery = Delivery::new(mailpit.api(), home.gateway.clone(), home.dir.path()).unwrap();
    assert!(delivery.deliver("recover").await.is_err());
    assert!(mailpit.state.lock().unwrap().deleted.is_empty());
    assert!(
        mailpit
            .state
            .lock()
            .unwrap()
            .messages
            .contains_key("recover")
    );
    home.matrix.state().refuse_rooms = false;
    drop(delivery);
    let mut recovered =
        Delivery::new(mailpit.api(), home.gateway.clone(), home.dir.path()).unwrap();
    recovered.deliver("recover").await.unwrap();
    assert_eq!(mailpit.state.lock().unwrap().deleted, ["recover"]);
    let matrix = home.matrix.state();
    assert_eq!(matrix.sent.len(), 1);
    assert!(matrix.sent[0].1.get(MAIL_FIELD).is_some());
}

#[tokio::test]
async fn notify_and_mail_share_the_same_quota_and_only_the_first_drop_has_a_hint() {
    let home = home().await;
    let mailpit = Mailpit::start().await;
    let plugin = crate::plugin::PluginId::parse(PLUGIN).unwrap();
    for _ in 0..crate::gateway::notify_limit::PER_MINUTE {
        assert!(home.gateway.notify_limiter.admit(&plugin).is_ok());
    }
    let mut delivery = Delivery::new(mailpit.api(), home.gateway.clone(), home.dir.path()).unwrap();
    for id in ["limited-1", "limited-2"] {
        mailpit.add(id, owner(), received("172.30.1.5"));
        delivery.deliver(id).await.unwrap();
    }
    let matrix = home.matrix.state();
    assert_eq!(matrix.sent.len(), 1);
    assert_eq!(matrix.sent[0].1["msgtype"], "m.notice");
    assert!(matrix.sent[0].1.get(MAIL_FIELD).is_none());
    assert_eq!(
        mailpit.state.lock().unwrap().deleted,
        ["limited-1", "limited-2"]
    );
}

#[tokio::test]
async fn a_delete_failure_does_not_resend_already_delivered_people_after_restart() {
    let home = home().await;
    let mailpit = Mailpit::start().await;
    mailpit.add("delete-later", owner(), received("172.30.1.5"));
    mailpit.state.lock().unwrap().fail_delete = true;
    let mut delivery = Delivery::new(mailpit.api(), home.gateway.clone(), home.dir.path()).unwrap();
    assert!(delivery.deliver("delete-later").await.is_err());
    assert_eq!(home.matrix.state().sent.len(), 1);
    drop(delivery);
    mailpit.state.lock().unwrap().fail_delete = false;
    let mut recovered =
        Delivery::new(mailpit.api(), home.gateway.clone(), home.dir.path()).unwrap();
    recovered.deliver("delete-later").await.unwrap();
    assert_eq!(home.matrix.state().sent.len(), 1);
    assert_eq!(mailpit.state.lock().unwrap().deleted, ["delete-later"]);
}

#[tokio::test]
async fn admitted_mail_keeps_its_original_sender_after_uninstall_or_subnet_reuse() {
    for reused in [false, true] {
        let home = home().await;
        let mailpit = Mailpit::start().await;
        mailpit.add("sender-drift", owner(), received("172.30.1.5"));
        home.matrix.state().refuse_rooms = true;
        let mut delivery =
            Delivery::new(mailpit.api(), home.gateway.clone(), home.dir.path()).unwrap();
        assert!(delivery.deliver("sender-drift").await.is_err());
        drop(delivery);
        home.gateway.plugins.register_source_for_test(
            PLUGIN,
            "172.30.2.0/24",
            vec![crate::plugin::Permission::Notify],
        );
        if reused {
            home.gateway.plugins.register_source_for_test(
                "app.test.other",
                "172.30.1.0/24",
                vec![crate::plugin::Permission::Notify],
            );
        }
        home.matrix.state().refuse_rooms = false;
        let mut recovered =
            Delivery::new(mailpit.api(), home.gateway.clone(), home.dir.path()).unwrap();
        recovered.deliver("sender-drift").await.unwrap();
        let matrix = home.matrix.state();
        assert_eq!(matrix.sent.len(), 1);
        assert_eq!(matrix.sent[0].1[MAIL_FIELD]["plugin_id"], PLUGIN);
        assert_eq!(matrix.sent[0].1[MAIL_FIELD]["plugin_name"], "Mail app");
        assert_eq!(mailpit.state.lock().unwrap().deleted, ["sender-drift"]);
    }
}
