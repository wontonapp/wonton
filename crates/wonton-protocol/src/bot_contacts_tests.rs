use serde_json::json;

use super::*;
use crate::{Action, BotContactAction};

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
        kind: ContactKind::Bot,
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
        "kind": "bot",
        "state": "active",
    }))
    .unwrap();
    assert_eq!(local.remote, None);

    for model in [
        BotContactsViewModel {
            bot: Some(bot()),
            state: BotContactsState::Ready,
            contacts: vec![remote_contact(), local],
            action_failure: Some(BotContactFailure {
                error: ContactError::InvalidUserId,
                message: "不是完整的 Matrix 用户 ID".into(),
            }),
        },
        BotContactsViewModel {
            bot: Some(bot()),
            state: BotContactsState::Failed {
                error: ContactError::NotPaired,
                message: "这台服务器还没连接".into(),
            },
            contacts: Vec::new(),
            action_failure: None,
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
        Action::BotContacts {
            action: BotContactAction::RefreshBotContacts { bot: bot() },
        },
        Action::BotContacts {
            action: BotContactAction::RequestBotContact {
                bot: bot(),
                user_id: user_id.clone(),
            },
        },
        Action::BotContacts {
            action: BotContactAction::RequestPersonContact {
                bot: bot(),
                user_id: "@sam:def67890.wonton".into(),
            },
        },
        Action::BotContacts {
            action: BotContactAction::ApproveBotContact {
                bot: bot(),
                user_id: user_id.clone(),
            },
        },
        Action::BotContacts {
            action: BotContactAction::RejectBotContact {
                bot: bot(),
                user_id: user_id.clone(),
            },
        },
        Action::BotContacts {
            action: BotContactAction::RemoveBotContact {
                bot: bot(),
                user_id,
            },
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
        "kind": "team",
        "state": "active",
        "remote": {"server_name": "def67890.wonton", "peer_status": "hibernating"},
    }))
    .unwrap();
    assert_eq!(contact.kind, ContactKind::Other);
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
        ContactError::PeerUnreachable,
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

/// 契约：联系人请求跟确认卡片走同一份列表和同一个通知类别；没有请求的旧列表照样解析。
#[test]
fn contact_requests_ride_the_confirmation_list_and_push() {
    let pending = crate::PendingContactRequest {
        bot: bot(),
        contact: remote_contact(),
        received_at_ms: 1_799_000_300_000,
    };
    let list: crate::ConfirmationList = serde_json::from_value(json!({
        "confirmations": [],
        "contact_requests": [serde_json::to_value(&pending).unwrap()],
    }))
    .unwrap();
    assert_eq!(list.contact_requests, [pending]);
    let old: crate::ConfirmationList =
        serde_json::from_value(json!({"confirmations": []})).unwrap();
    assert!(old.contact_requests.is_empty());

    let push = crate::ContactRequestPush {
        bot: bot(),
        user_id: "@hermes-sam:def67890.wonton".into(),
        display_name: Some("Hermes".into()),
        keeper_display_name: Some("Sam".into()),
    };
    let notification = crate::PushNotification {
        server_id: Some("a1".into()),
        room_id: String::new(),
        event_id: String::new(),
        sender: None,
        room_name: None,
        content: crate::PushContent::Invite,
        category: crate::NotificationCategory::Confirmation,
        confirmation: None,
        contact_request: Some(push),
        thread_root: None,
        hidden: false,
        lock_screen_private: false,
        unread_total: 1,
    };
    let value = serde_json::to_value(&notification).unwrap();
    assert_eq!(
        serde_json::from_value::<crate::PushNotification>(value).unwrap(),
        notification
    );
    let message = crate::PushNotification {
        contact_request: None,
        thread_root: None,
        hidden: false,
        ..notification
    };
    assert!(
        serde_json::to_value(&message)
            .unwrap()
            .get("contact_request")
            .is_none()
    );
}

/// 契约：请人做联系人的请求体和私聊邀请的标记。标记放在 create 内容里，和好友请求同一种做法。
#[test]
fn a_person_is_asked_with_a_marked_direct_invite() {
    let request: ContactRequest =
        serde_json::from_value(json!({"user_id": "@sam:def67890.wonton", "kind": "person"}))
            .unwrap();
    assert_eq!(request.kind, ContactKind::Person);
    assert_ne!(BOT_CONTACT_CREATE_KEY, crate::FRIEND_EVENT);
    assert_eq!(
        serde_json::from_value::<crate::FriendRequestKind>(json!("bot_contact")).unwrap(),
        crate::FriendRequestKind::BotContact
    );
}
