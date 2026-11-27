//! 读 Bot 记录与联系人事实。

use super::*;

#[tokio::test]
async fn exact_bot_and_full_person_id_select_the_recorded_room() {
    let mut fixture = Fixture::new().await;
    let alice = fixture.member("alice", "@alice:test");
    let bob = fixture.member("bob", "@bob:test");
    fixture.seed(&[
        (
            "alice",
            "@private-a:test",
            BotKeeper::User {
                username: "alice".into(),
            },
            "!home-a:test",
        ),
        (
            "bob",
            "@private-b:test",
            BotKeeper::User {
                username: "bob".into(),
            },
            "!home-b:test",
        ),
    ]);
    fixture.contacts(contact_rows(&bot("alice"), json!({
        "@alice:test": {"kind": "person", "state": "active", "room_id": "!person-a:test", "updated_at_ms": 1},
        "@alice:elsewhere.test": {"kind": "person", "state": "active", "room_id": "!wrong-server:test", "updated_at_ms": 1},
        "@bob:test": {"kind": "person", "state": "pending", "room_id": "!person-b:test", "updated_at_ms": 1}
    })));
    let facts = fixture.facts(Some(alice), &bot("alice"));
    let record = facts.bot.unwrap();
    assert_eq!(record.listing.bot, bot("alice"));
    assert_eq!(record.listing.user_id, "@private-a:test");
    assert_eq!(
        record.listing.keeper,
        BotKeeper::User {
            username: "alice".into()
        }
    );
    assert_eq!(record.home_room, "!home-a:test");
    assert_eq!(
        facts.caller,
        CallerFacts::Known {
            source: CallerSource::PairedDevice,
            person: Person::member("alice"),
            user_id: Some("@alice:test".into())
        }
    );
    assert_eq!(
        facts.contact,
        PersonContactFacts::Record(PersonContactRecordRead::Person {
            state: ContactState::Active,
            room_id: Some("!person-a:test".into())
        })
    );
    assert_eq!(
        fixture.facts(Some(bob), &bot("alice")).contact,
        PersonContactFacts::Record(PersonContactRecordRead::Person {
            state: ContactState::Pending,
            room_id: Some("!person-b:test".into())
        })
    );
    let other = fixture.facts(Some(alice), &bot("bob"));
    assert_eq!(other.bot.unwrap().listing.user_id, "@private-b:test");
    assert_eq!(
        other.contact,
        PersonContactFacts::Record(PersonContactRecordRead::Missing)
    );
}

#[tokio::test]
async fn missing_source_unknown_identity_and_missing_exact_row_remain_distinct() {
    let mut fixture = Fixture::new().await;
    let alice = fixture.member("alice", "@alice:test");
    let reference = bot("shared");
    fixture.seed(&[(
        "shared",
        "@shared:test",
        BotKeeper::Household,
        "!shared-home:test",
    )]);
    assert_eq!(
        fixture.facts(Some(alice), &reference).contact,
        PersonContactFacts::NotConnected
    );
    fixture.contacts(contact_rows(
        &reference,
        json!({
            "alice": {"kind": "person", "state": "active", "updated_at_ms": 1},
            "@alice:elsewhere.test": {"kind": "person", "state": "active", "updated_at_ms": 1}
        }),
    ));
    assert_eq!(
        fixture.facts(Some(alice), &reference).contact,
        PersonContactFacts::Record(PersonContactRecordRead::Missing)
    );
    let without_peer = fixture.facts(None, &reference);
    assert_eq!(without_peer.caller, CallerFacts::Unknown);
    assert_eq!(without_peer.contact, PersonContactFacts::UnknownIdentity);
    assert_eq!(
        without_peer.bot.unwrap().listing.keeper,
        BotKeeper::Household
    );
}

#[tokio::test]
async fn pending_and_unknown_states_are_not_refreshed_or_promoted() {
    let mut fixture = Fixture::new().await;
    let alice = fixture.member("alice", "@alice:test");
    let reference = bot("shared");
    fixture.seed(&[("shared", "@shared:test", BotKeeper::Household, "!home:test")]);
    let bot_bytes = std::fs::read(fixture.record_path(PLUGIN)).unwrap();
    for (raw, expected) in [
        ("pending", ContactState::Pending),
        ("incoming", ContactState::Incoming),
        ("active", ContactState::Active),
        ("declined", ContactState::Declined),
        ("not_found", ContactState::NotFound),
        ("future_state", ContactState::Other),
    ] {
        let (invites, effects, path) = fixture.contacts(contact_rows(&reference, json!({
            "@alice:test": {"kind": "person", "state": raw, "room_id": "!exact:test", "updated_at_ms": 1}
        })));
        invites
            .states
            .lock()
            .unwrap()
            .insert("!exact:test".into(), crate::contacts::InviteeState::Joined);
        let bytes = std::fs::read(&path).unwrap();
        let contacts = fixture.gateway.contacts.as_ref().unwrap();
        let changes = contacts.changes();
        let before = contacts.changes_count();
        assert_eq!(
            fixture.facts(Some(alice), &reference).contact,
            PersonContactFacts::Record(PersonContactRecordRead::Person {
                state: expected,
                room_id: Some("!exact:test".into())
            })
        );
        assert_eq!(contacts.changes_count(), before);
        assert!(!changes.has_changed().unwrap());
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        assert_eq!(
            std::fs::read(fixture.record_path(PLUGIN)).unwrap(),
            bot_bytes
        );
        assert!(invites.sent.lock().unwrap().is_empty());
        assert!(invites.withdrawn.lock().unwrap().is_empty());
        assert!(effects.rebuilt.lock().unwrap().is_empty());
        assert!(effects.pushed.lock().unwrap().is_empty());
    }
}

#[tokio::test]
async fn non_person_kinds_and_absent_or_empty_rooms_are_not_contact_grants() {
    let mut fixture = Fixture::new().await;
    let alice = fixture.member("alice", "@alice:test");
    let reference = bot("shared");
    fixture.seed(&[(
        "shared",
        "@shared:test",
        BotKeeper::Household,
        "!home-must-not-fill-room:test",
    )]);
    for (kind, expected) in [
        ("bot", PersonContactRecordRead::NotPerson),
        ("future_kind", PersonContactRecordRead::UnknownKind),
    ] {
        fixture.contacts(contact_rows(&reference, json!({
            "@alice:test": {"kind": kind, "state": "active", "room_id": "!not-person:test", "updated_at_ms": 1}
        })));
        assert_eq!(
            fixture.facts(Some(alice), &reference).contact,
            PersonContactFacts::Record(expected)
        );
    }
    for room in [None, Some("")] {
        fixture.contacts(contact_rows(&reference, json!({
            "@alice:test": {"kind": "person", "state": "active", "room_id": room, "updated_at_ms": 1}
        })));
        assert_eq!(
            fixture.facts(Some(alice), &reference).contact,
            PersonContactFacts::Record(PersonContactRecordRead::Person {
                state: ContactState::Active,
                room_id: room.map(str::to_owned)
            })
        );
    }
}

#[tokio::test]
async fn missing_bot_and_unknown_keeper_do_not_hide_real_store_errors() {
    let fixture = Fixture::new().await;
    fixture.seed(&[("recorded", "@opaque:test", BotKeeper::Other, "")]);
    let path = fixture.record_path(PLUGIN);
    std::fs::write(fixture.record_path("app.unrelated"), b"{broken").unwrap();
    let record = fixture.facts(None, &bot("recorded")).bot.unwrap();
    assert_eq!(record.listing.keeper, BotKeeper::Other);
    assert_eq!(record.home_room, "");
    assert_eq!(record.listing.user_id, "@opaque:test");
    assert_eq!(fixture.facts(None, &bot("missing")).bot, None);
    let missing_plugin = BotRef {
        plugin_id: "app.absent".into(),
        bot: "recorded".into(),
    };
    assert_eq!(fixture.facts(None, &missing_plugin).bot, None);
    std::fs::write(&path, b"{broken").unwrap();
    assert!(
        matches!(bot_identity_and_contact_facts(&fixture.gateway, None, &HeaderMap::new(), &bot("recorded")),
        Err(StoreError::Corrupt { path: actual, .. }) if actual == path)
    );
    std::fs::remove_file(&path).unwrap();
    std::fs::create_dir(&path).unwrap();
    assert!(
        matches!(bot_identity_and_contact_facts(&fixture.gateway, None, &HeaderMap::new(), &bot("recorded")),
        Err(StoreError::Io { path: actual, .. }) if actual == path)
    );
}
