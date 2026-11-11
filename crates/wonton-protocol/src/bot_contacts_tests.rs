use serde_json::json;

use super::*;
use crate::Action;

fn bot() -> BotRef {
    BotRef {
        plugin_id: "ai.nous.hermes".into(),
        bot: "alice".into(),
    }
}

fn remote_contact() -> BotContact {
    BotContact {
        user_id: "@hermes-sam:def67890.wonton".into(),
        display_name: Some("Sam 的 Hermes".into()),
        keeper_display_name: Some("Sam".into()),
        state: ContactState::Incoming,
        remote: Some(RemoteContact {
            server_name: "def67890.wonton".into(),
            peer_status: Some(PeerStatus::Offline),
        }),
    }
}

#[test]
fn bots_are_addressed_by_plugin_and_bot_name() {
    let user = "@hermes-bob:x.wonton.internal";
    assert_eq!(
        bot_contact_path(&bot(), user),
        "/_wonton/api/bots/ai.nous.hermes/alice/contacts/@hermes-bob:x.wonton.internal"
    );
    assert_eq!(
        bot_contact_approve_path(&bot(), user),
        format!("{}/approve", bot_contact_path(&bot(), user))
    );
    assert_eq!(
        bot_contact_reject_path(&bot(), user),
        format!("{}/reject", bot_contact_path(&bot(), user))
    );
}

/// 契约：列表、ViewModel 和操作都能经 JSON 往返；旧核心不给 `remote` 时当同机。
#[test]
fn contact_types_round_trip_through_json() {
    let contacts = BotContacts {
        contacts: vec![remote_contact()],
    };
    let json = serde_json::to_string(&contacts).unwrap();
    assert_eq!(
        serde_json::from_str::<BotContacts>(&json).unwrap(),
        contacts
    );

    let local: BotContact = serde_json::from_value(json!({
        "user_id": "@hermes-bob:x.wonton.internal",
        "display_name": null,
        "keeper_display_name": "Bob",
        "state": "active",
    }))
    .unwrap();
    assert_eq!(local.remote, None);

    for model in [
        BotContactsViewModel {
            bot: Some(bot()),
            state: BotContactsState::Ready,
            contacts: vec![remote_contact(), local],
        },
        BotContactsViewModel {
            bot: Some(bot()),
            state: BotContactsState::Failed {
                error: ContactError::NotPaired,
                message: "这台服务器还没连接".into(),
            },
            contacts: Vec::new(),
        },
    ] {
        let json = serde_json::to_string(&model).unwrap();
        assert_eq!(
            serde_json::from_str::<BotContactsViewModel>(&json).unwrap(),
            model
        );
    }

    let user_id = "@hermes-sam:def67890.wonton".to_owned();
    for action in [
        Action::RefreshBotContacts { bot: bot() },
        Action::RequestBotContact {
            bot: bot(),
            user_id: user_id.clone(),
        },
        Action::ApproveBotContact {
            bot: bot(),
            user_id: user_id.clone(),
        },
        Action::RejectBotContact {
            bot: bot(),
            user_id: user_id.clone(),
        },
        Action::RemoveBotContact {
            bot: bot(),
            user_id,
        },
    ] {
        let json = serde_json::to_string(&action).unwrap();
        assert_eq!(serde_json::from_str::<Action>(&json).unwrap(), action);
    }
}

/// 契约：两台核心之间的事件写法。版本不同的核心靠它互通，改写法要两边一起升级。
#[test]
fn contact_events_are_written_as_the_other_core_reads_them() {
    let request = ContactEvent::Request {
        id: "c1".into(),
        from: "@hermes-alice:abc12345.wonton".into(),
        to: "@hermes-sam:def67890.wonton".into(),
        from_display_name: Some("Hermes".into()),
        keeper_display_name: Some("Alice".into()),
    };
    assert_eq!(
        serde_json::to_value(&request).unwrap(),
        json!({
            "type": "request",
            "id": "c1",
            "from": "@hermes-alice:abc12345.wonton",
            "to": "@hermes-sam:def67890.wonton",
            "from_display_name": "Hermes",
            "keeper_display_name": "Alice",
        })
    );

    let result = ContactEvent::Result {
        id: "c1".into(),
        from: "@hermes-sam:def67890.wonton".into(),
        to: "@hermes-alice:abc12345.wonton".into(),
        outcome: ContactOutcome::Approved,
        keeper_display_name: None,
    };
    assert_eq!(
        serde_json::to_value(&result).unwrap(),
        json!({
            "type": "result",
            "id": "c1",
            "from": "@hermes-sam:def67890.wonton",
            "to": "@hermes-alice:abc12345.wonton",
            "outcome": "approved",
            "keeper_display_name": null,
        })
    );

    let remove = ContactEvent::Remove {
        from: "@hermes-alice:abc12345.wonton".into(),
        to: "@hermes-sam:def67890.wonton".into(),
    };
    for event in [request, result, remove] {
        let value = serde_json::to_value(&event).unwrap();
        assert_eq!(
            serde_json::from_value::<ContactEvent>(value).unwrap(),
            event
        );
    }
}

/// 契约：比自己新的核心或 App 多出的取值落到 `Other`，不让整条解析失败。
#[test]
fn values_a_newer_peer_adds_fall_back_to_other() {
    assert_eq!(
        serde_json::from_value::<ContactState>(json!("blocked")).unwrap(),
        ContactState::Other
    );
    assert_eq!(
        serde_json::from_value::<ContactOutcome>(json!("deferred")).unwrap(),
        ContactOutcome::Other
    );
    assert_eq!(
        serde_json::from_value::<ContactEvent>(json!({"type": "tools", "from": "@a:b"})).unwrap(),
        ContactEvent::Other
    );
    assert_eq!(
        serde_json::from_value::<BotContactsState>(json!({"type": "paused"})).unwrap(),
        BotContactsState::Other
    );
    let contact: BotContact = serde_json::from_value(json!({
        "user_id": "@hermes-sam:def67890.wonton",
        "display_name": null,
        "keeper_display_name": null,
        "state": "active",
        "remote": {"server_name": "def67890.wonton", "peer_status": "hibernating"},
    }))
    .unwrap();
    assert_eq!(contact.remote.unwrap().peer_status, Some(PeerStatus::Other));
}

/// 契约：类别的写法就是错误头和错误体 `kind` 的值；不认识的落到 `Other`。
#[test]
fn error_kinds_are_wire_strings() {
    for error in [
        ContactError::InvalidUserId,
        ContactError::NotPaired,
        ContactError::AlreadyContact,
        ContactError::BotNotFound,
        ContactError::ContactNotFound,
        ContactError::NotKeeper,
        ContactError::Unauthenticated,
        ContactError::Upstream,
        ContactError::TransportUnavailable,
        ContactError::Protocol,
        ContactError::Other,
    ] {
        assert_eq!(serde_json::to_value(error).unwrap(), json!(error.as_str()));
    }
    assert_eq!(
        serde_json::from_value::<ContactError>(json!("quota")).unwrap(),
        ContactError::Other
    );
    let body: ContactErrorBody = serde_json::from_value(json!({
        "error": {"kind": "not_paired", "message": "这台服务器还没连接"}
    }))
    .unwrap();
    assert_eq!(body.error.kind, ContactError::NotPaired);
    assert_eq!(ContactError::NotPaired.http_status(), Some(403));
    assert_eq!(ContactError::TransportUnavailable.http_status(), None);
}
