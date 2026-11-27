//! 按联系人记录判聊天可用性。

use super::*;

#[tokio::test]
async fn a_person_contact_grant_toggles_chat_availability() {
    let mut fixture = Fixture::new().await;
    let mia = fixture.member("mia", "@mia:test");
    let bot = bot("default");
    fixture.seed(&[(
        "default",
        "@private:test",
        BotKeeper::User {
            username: "owner".into(),
        },
        "!home:test",
    )]);
    let facts = fixture.facts(Some(mia), &bot);
    assert_eq!(
        chat_availability(&fixture.gateway, &facts),
        BotChatAvailability::Unavailable {
            reason: BotChatUnavailable::OwnerPrivate
        }
    );

    // 表在、只有别人的记录、没有 mia 的行：从没授权过，不能说“已经收回”。
    fixture.contacts(contact_rows(
        &bot,
        json!({"@someone-else:test": {"kind": "person", "state": "active", "updated_at_ms": 1}}),
    ));
    let facts = fixture.facts(Some(mia), &bot);
    assert_eq!(
        facts.contact,
        PersonContactFacts::Record(PersonContactRecordRead::Missing)
    );
    assert_eq!(
        chat_availability(&fixture.gateway, &facts),
        BotChatAvailability::Unavailable {
            reason: BotChatUnavailable::OwnerPrivate
        }
    );

    for state in ["pending", "incoming"] {
        fixture.contacts(contact_rows(
            &bot,
            json!({"@mia:test": {"kind": "person", "state": state, "updated_at_ms": 1}}),
        ));
        let facts = fixture.facts(Some(mia), &bot);
        assert_eq!(
            chat_availability(&fixture.gateway, &facts),
            BotChatAvailability::Unavailable {
                reason: BotChatUnavailable::ContactPending,
            },
            "{state}"
        );
    }

    fixture.contacts(contact_rows(
        &bot,
        json!({"@mia:test": {"kind": "person", "state": "active", "updated_at_ms": 1}}),
    ));
    let facts = fixture.facts(Some(mia), &bot);
    assert_eq!(
        chat_availability(&fixture.gateway, &facts),
        BotChatAvailability::Available
    );

    for state in ["declined", "not_found"] {
        fixture.contacts(contact_rows(
            &bot,
            json!({"@mia:test": {"kind": "person", "state": state, "updated_at_ms": 2}}),
        ));
        let facts = fixture.facts(Some(mia), &bot);
        assert_eq!(
            chat_availability(&fixture.gateway, &facts),
            BotChatAvailability::Unavailable {
                reason: BotChatUnavailable::ContactDeclined
            },
            "{state}"
        );
    }
}

#[tokio::test]
async fn a_missing_record_is_bot_missing_for_an_unknown_caller_too() {
    let fixture = Fixture::new().await;
    let facts = fixture.facts(None, &bot("missing"));
    assert_eq!(
        chat_availability(&fixture.gateway, &facts),
        BotChatAvailability::Unavailable {
            reason: BotChatUnavailable::BotMissing
        }
    );
}

#[tokio::test]
async fn ownership_and_role_decide_own_shared_and_other_bots() {
    let fixture = Fixture::new().await;
    let alice = fixture.member("alice", "@alice:test");
    fixture.seed(&[
        (
            "alice",
            "@alice-private:test",
            BotKeeper::User {
                username: "alice".into(),
            },
            "!alice-home:test",
        ),
        (
            "shared",
            "@shared:test",
            BotKeeper::Household,
            "!shared-home:test",
        ),
        ("future", "@future:test", BotKeeper::Other, ""),
    ]);
    let availability = |peer: TunnelPeer, reference: &BotRef| {
        chat_availability(&fixture.gateway, &fixture.facts(Some(peer), reference))
    };
    assert_eq!(
        availability(alice, &bot("alice")),
        BotChatAvailability::Available
    );
    // 默认邀请角色不给共享 Bot，换成家人角色就给。
    assert_eq!(
        availability(alice, &bot("shared")),
        BotChatAvailability::Unavailable {
            reason: BotChatUnavailable::RoleLacksSharedBots
        }
    );
    fixture.gateway.members.set_role("alice", "family").unwrap();
    assert_eq!(
        availability(alice, &bot("shared")),
        BotChatAvailability::Available
    );
    assert_eq!(
        availability(alice, &bot("future")),
        BotChatAvailability::Unknown
    );
}
