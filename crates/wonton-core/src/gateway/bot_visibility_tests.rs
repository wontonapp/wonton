//! `bot_visibility` 的分类和事实读取测试。

use std::path::PathBuf;
use std::sync::Arc;

use futures_util::future::BoxFuture;
use hyper::HeaderMap;
use hyper::header::{AUTHORIZATION, ORIGIN};
use serde_json::{Value, json};
use wonton_protocol::{BotKeeper, ContactEvent, ContactState};

use super::*;
use crate::contacts::tests::{FakeDirectory, FakeEffects, FakeInvites};
use crate::contacts::{ContactFailure, Contacts, PeerLink, PeerReach, Ports};
use crate::gateway::tests::{Harness, TOKEN};
use crate::http::HttpClient;
use crate::matrix::{BotAccount, Matrix};
use crate::members::MemberRecord;
use crate::plugin::bots::{BotEntry, BotStore, PluginBots};
use crate::plugin::manifest::PluginId;

const PLUGIN: &str = "ai.nous.hermes";

struct NoLink;

impl PeerLink for NoLink {
    fn reach(&self, _: &str) -> PeerReach {
        unreachable!("原始事实读取不查询联邦连接")
    }

    fn send(&self, _: &str, _: ContactEvent) -> BoxFuture<'_, Result<(), ContactFailure>> {
        unreachable!("原始事实读取不发送联邦事件")
    }
}

struct Fixture {
    gateway: Gateway,
    harness: Harness,
}

impl Fixture {
    async fn new() -> Self {
        let harness = Harness::start(None).await;
        let gateway = Gateway::new(
            harness.gateway.public.clone(),
            harness.gateway.owner.clone(),
            TOKEN.into(),
            harness.members.clone(),
            harness.manager.clone(),
            None,
            harness.http.clone(),
            harness.tunnel.clone(),
        );
        Self { gateway, harness }
    }

    fn peer(&self, username: Option<&str>) -> TunnelPeer {
        let id = iroh::SecretKey::generate().public();
        self.gateway
            .tunnel
            .devices
            .add_if(id, None, username.map(str::to_owned), |_| true)
            .unwrap();
        TunnelPeer(id)
    }

    fn member(&self, username: &str, user_id: &str) -> TunnelPeer {
        let mut record = MemberRecord::for_test(username);
        record.user_id = user_id.into();
        self.gateway.members.invite(record).unwrap();
        self.gateway.members.mark_joined(username).unwrap();
        self.peer(Some(username))
    }

    fn seed(&self, entries: &[(&str, &str, BotKeeper, &str)]) {
        let bots = entries
            .iter()
            .map(|(name, user_id, keeper, room)| {
                (
                    name.to_string(),
                    BotEntry {
                        account: BotAccount {
                            user_id: user_id.to_string(),
                            home_room: room.to_string(),
                        },
                        device_id: None,
                        keeper: keeper.clone(),
                        bus_token: "isolated-test-bus-token".into(),
                    },
                )
            })
            .collect();
        BotStore::new(self.harness.data_dir.join("plugins/bots"))
            .save(&PluginId::parse(PLUGIN).unwrap(), &PluginBots { bots })
            .unwrap();
    }

    fn record_path(&self, plugin: &str) -> PathBuf {
        self.harness
            .data_dir
            .join("plugins/bots")
            .join(format!("{plugin}.json"))
    }

    fn contacts(&mut self, rows: Value) -> (Arc<FakeInvites>, Arc<FakeEffects>, PathBuf) {
        let path = self.harness.data_dir.join("raw-contact-facts.json");
        std::fs::write(&path, serde_json::to_vec(&json!({"bots": rows})).unwrap()).unwrap();
        let invites = Arc::new(FakeInvites::default());
        let effects = Arc::new(FakeEffects::default());
        self.gateway.contacts = Some(
            Contacts::open(
                &path,
                "test",
                Ports {
                    directory: Arc::new(FakeDirectory::default()),
                    link: Arc::new(NoLink),
                    invites: invites.clone(),
                    effects: effects.clone(),
                    open_federation: false,
                },
            )
            .unwrap(),
        );
        (invites, effects, path)
    }

    fn facts(&self, peer: Option<TunnelPeer>, bot: &BotRef) -> BotIdentityAndContactFacts {
        bot_identity_and_contact_facts(&self.gateway, peer, &HeaderMap::new(), bot).unwrap()
    }
}

fn bot(name: &str) -> BotRef {
    BotRef {
        plugin_id: PLUGIN.into(),
        bot: name.into(),
    }
}

fn contact_rows(bot: &BotRef, contacts: Value) -> Value {
    json!({
        (format!("{}/{}", bot.plugin_id, bot.bot)): {"bot": bot, "contacts": contacts}
    })
}

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
            reason: BotChatUnavailable::OtherPrivateBot
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
            reason: BotChatUnavailable::OtherPrivateBot
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
                reason: BotChatUnavailable::OtherPrivateBot
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
                reason: BotChatUnavailable::GrantNotActive
            },
            "{state}"
        );
    }
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
            reason: BotChatUnavailable::Other
        }
    );
    fixture.gateway.members.set_role("alice", "family").unwrap();
    assert_eq!(
        availability(alice, &bot("shared")),
        BotChatAvailability::Available
    );
    assert_eq!(
        availability(alice, &bot("future")),
        BotChatAvailability::Unavailable {
            reason: BotChatUnavailable::Other
        }
    );
}

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

#[tokio::test]
async fn revocation_and_member_removal_are_requeried_for_the_original_peer() {
    let mut fixture = Fixture::new().await;
    let alice = fixture.member("alice", "@alice:test");
    let bob = fixture.member("bob", "@bob:test");
    let reference = bot("shared");
    fixture.seed(&[("shared", "@shared:test", BotKeeper::Household, "!home:test")]);
    fixture.contacts(json!({}));
    assert_eq!(
        fixture.facts(Some(alice), &reference).caller,
        CallerFacts::Known {
            source: CallerSource::PairedDevice,
            person: Person::member("alice"),
            user_id: Some("@alice:test".into())
        }
    );
    fixture.gateway.tunnel.devices.remove(&alice.0).unwrap();
    let revoked = fixture.facts(Some(alice), &reference);
    assert_eq!(revoked.caller, CallerFacts::Unknown);
    assert_eq!(revoked.contact, PersonContactFacts::UnknownIdentity);
    fixture.gateway.members.remove("bob", false, false).unwrap();
    assert!(fixture.gateway.tunnel.devices.get(&bob.0).is_some());
    let removed = fixture.facts(Some(bob), &reference);
    assert_eq!(removed.caller, CallerFacts::Unknown);
    assert_eq!(removed.contact, PersonContactFacts::UnknownIdentity);
    assert_eq!(
        fixture
            .facts(Some(fixture.peer(Some("unrecorded"))), &reference)
            .caller,
        CallerFacts::Unknown
    );
}

#[tokio::test]
async fn absent_or_inconsistent_matrix_ids_do_not_fall_back_to_the_public_domain() {
    let mut fixture = Fixture::new().await;
    let reference = bot("shared");
    fixture.seed(&[("shared", "@shared:test", BotKeeper::Household, "!home:test")]);
    fixture.contacts(json!({}));
    let owner = fixture.peer(None);
    assert_eq!(
        fixture.facts(Some(owner), &reference).caller,
        CallerFacts::Known {
            source: CallerSource::PairedDevice,
            person: Person::owner("owner"),
            user_id: None
        }
    );
    for (username, raw) in [
        ("alice", ""),
        ("bob", "@someone-else:test"),
        ("carol", "not-an-id"),
    ] {
        let peer = fixture.member(username, raw);
        let facts = fixture.facts(Some(peer), &reference);
        assert_eq!(
            facts.caller,
            CallerFacts::Known {
                source: CallerSource::PairedDevice,
                person: Person::member(username),
                user_id: None
            }
        );
        assert_eq!(facts.contact, PersonContactFacts::UnknownIdentity);
    }
    let full_id = "@dave:recorded.example.test:8448";
    let dave = fixture.member("dave", full_id);
    assert_eq!(
        fixture.facts(Some(dave), &reference).caller,
        CallerFacts::Known {
            source: CallerSource::PairedDevice,
            person: Person::member("dave"),
            user_id: Some(full_id.into())
        }
    );
}

#[tokio::test]
async fn configured_matrix_not_the_access_domain_is_the_owner_identity_source() {
    let mut fixture = Fixture::new().await;
    let (_, config) = crate::matrix::fake::start().await;
    let state = tempfile::tempdir().unwrap();
    let matrix = Matrix::connect(&config, state.path(), HttpClient::new())
        .await
        .unwrap();
    let owner_id = matrix.user_id("owner");
    let alice_id = matrix.user_id("alice");
    fixture.gateway.public.base_domain = "access-only.invalid".into();
    fixture.gateway.matrix = Some(Arc::new(matrix));
    fixture.contacts(json!({}));
    let owner = fixture.peer(None);
    let alice = fixture.member("alice", &alice_id);
    let bob = fixture.member("bob", "@bob:elsewhere.test");
    assert_eq!(
        fixture.facts(Some(owner), &bot("absent")).caller,
        CallerFacts::Known {
            source: CallerSource::PairedDevice,
            person: Person::owner("owner"),
            user_id: Some(owner_id)
        }
    );
    assert_eq!(
        fixture.facts(Some(alice), &bot("absent")).caller,
        CallerFacts::Known {
            source: CallerSource::PairedDevice,
            person: Person::member("alice"),
            user_id: Some(alice_id)
        }
    );
    let foreign = fixture.facts(Some(bob), &bot("absent"));
    assert_eq!(
        foreign.caller,
        CallerFacts::Known {
            source: CallerSource::PairedDevice,
            person: Person::member("bob"),
            user_id: None
        }
    );
    assert_eq!(foreign.contact, PersonContactFacts::UnknownIdentity);
}

#[tokio::test]
async fn request_auth_bearer_precedes_peers_without_device_origin_gate() {
    let mut fixture = Fixture::new().await;
    let (_, config) = crate::matrix::fake::start().await;
    let state = tempfile::tempdir().unwrap();
    let matrix = Matrix::connect(&config, state.path(), HttpClient::new())
        .await
        .unwrap();
    let owner_id = matrix.user_id("owner");
    let alice = fixture.member("alice", &matrix.user_id("alice"));
    fixture.gateway.matrix = Some(Arc::new(matrix));
    fixture.gateway.public.base_domain = "access-only.invalid".into();
    let unknown = TunnelPeer(iroh::SecretKey::generate().public());
    let reference = bot("default");
    fixture.seed(&[(
        "default",
        "@owner-agent:test",
        BotKeeper::User {
            username: "owner".into(),
        },
        "!home:test",
    )]);
    fixture.contacts(json!({}));
    for peer in [None, Some(alice), Some(unknown)] {
        for origin in [None, Some("https://another-page.invalid")] {
            let mut headers = HeaderMap::new();
            headers.insert(AUTHORIZATION, format!("Bearer {TOKEN}").parse().unwrap());
            if let Some(origin) = origin {
                headers.insert(ORIGIN, origin.parse().unwrap());
            }
            assert!(
                api::plugin_denied(&fixture.gateway, peer.map(axum::Extension), &headers).is_none()
            );
            let facts =
                bot_identity_and_contact_facts(&fixture.gateway, peer, &headers, &reference)
                    .unwrap();
            assert_eq!(
                facts.caller,
                CallerFacts::Known {
                    source: CallerSource::OwnerBearer,
                    person: Person::owner("owner"),
                    user_id: Some(owner_id.clone())
                }
            );
            assert_eq!(
                facts.contact,
                PersonContactFacts::Record(PersonContactRecordRead::Missing)
            );
        }
    }
}

#[tokio::test]
async fn request_auth_device_uses_current_mapping_and_existing_origin_gate() {
    let mut fixture = Fixture::new().await;
    let alice = fixture.member("alice", "@alice:test");
    let unknown = TunnelPeer(iroh::SecretKey::generate().public());
    let reference = bot("default");
    fixture.contacts(json!({}));
    let mut headers = HeaderMap::new();
    headers.insert(ORIGIN, fixture.gateway.core_origin().parse().unwrap());
    for authorization in [None, Some("Bearer invalid"), Some("Basic invalid")] {
        if let Some(value) = authorization {
            headers.insert(AUTHORIZATION, value.parse().unwrap());
        } else {
            headers.remove(AUTHORIZATION);
        }
        assert!(
            api::plugin_denied(&fixture.gateway, Some(axum::Extension(alice)), &headers).is_none()
        );
        let facts =
            bot_identity_and_contact_facts(&fixture.gateway, Some(alice), &headers, &reference)
                .unwrap();
        assert_eq!(
            facts.caller,
            CallerFacts::Known {
                source: CallerSource::PairedDevice,
                person: Person::member("alice"),
                user_id: Some("@alice:test".into())
            }
        );
        for peer in [None, Some(unknown)] {
            let denied =
                api::plugin_denied(&fixture.gateway, peer.map(axum::Extension), &headers).unwrap();
            assert_eq!(denied.status(), hyper::StatusCode::UNAUTHORIZED);
            assert_eq!(
                bot_identity_and_contact_facts(&fixture.gateway, peer, &headers, &reference)
                    .unwrap()
                    .caller,
                CallerFacts::Unknown
            );
        }
    }
    headers.insert(ORIGIN, "https://another-page.invalid".parse().unwrap());
    let denied =
        api::plugin_denied(&fixture.gateway, Some(axum::Extension(alice)), &headers).unwrap();
    assert_eq!(denied.status(), hyper::StatusCode::FORBIDDEN);
    let facts = bot_identity_and_contact_facts(&fixture.gateway, Some(alice), &headers, &reference)
        .unwrap();
    assert_eq!(facts.caller, CallerFacts::Unknown);
    assert_eq!(facts.contact, PersonContactFacts::UnknownIdentity);
}

#[tokio::test]
async fn request_auth_rechecks_revocation_and_propagates_store_errors() {
    let mut fixture = Fixture::new().await;
    let alice = fixture.member("alice", "@alice:test");
    let reference = bot("default");
    fixture.seed(&[("default", "@agent:test", BotKeeper::Household, "!home:test")]);
    fixture.contacts(contact_rows(&reference, json!({
        "@alice:test": {"kind": "person", "state": "active", "room_id": "!person:test", "updated_at_ms": 1}
    })));
    let before = fixture.facts(Some(alice), &reference);
    assert!(matches!(
        before.contact,
        PersonContactFacts::Record(PersonContactRecordRead::Person {
            state: ContactState::Active,
            ..
        })
    ));
    fixture.gateway.tunnel.devices.remove(&alice.0).unwrap();
    assert_eq!(
        fixture.facts(Some(alice), &reference).caller,
        CallerFacts::Unknown
    );
    let renewed = fixture.peer(Some("alice"));
    assert!(matches!(
        fixture.facts(Some(renewed), &reference).caller,
        CallerFacts::Known {
            source: CallerSource::PairedDevice,
            ..
        }
    ));
    fixture
        .gateway
        .members
        .remove("alice", false, false)
        .unwrap();
    assert!(fixture.gateway.tunnel.devices.get(&renewed.0).is_some());
    assert_eq!(
        fixture.facts(Some(renewed), &reference).caller,
        CallerFacts::Unknown
    );
    let mut headers = HeaderMap::new();
    headers.insert(AUTHORIZATION, format!("Bearer {TOKEN}").parse().unwrap());
    let facts =
        bot_identity_and_contact_facts(&fixture.gateway, Some(renewed), &headers, &reference)
            .unwrap();
    assert_eq!(
        facts.caller,
        CallerFacts::Known {
            source: CallerSource::OwnerBearer,
            person: Person::owner("owner"),
            user_id: None
        }
    );
    let path = fixture.record_path(PLUGIN);
    std::fs::write(&path, b"{broken").unwrap();
    assert!(
        matches!(bot_identity_and_contact_facts(&fixture.gateway, Some(renewed), &headers, &reference),
        Err(StoreError::Corrupt { path: actual, .. }) if actual == path)
    );
    std::fs::remove_file(&path).unwrap();
    std::fs::create_dir(&path).unwrap();
    assert!(
        matches!(bot_identity_and_contact_facts(&fixture.gateway, Some(renewed), &headers, &reference),
        Err(StoreError::Io { path: actual, .. }) if actual == path)
    );
}
