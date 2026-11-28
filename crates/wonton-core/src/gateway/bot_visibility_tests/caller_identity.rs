//! 从 peer 和成员记录读调用人身份。

use super::*;

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
