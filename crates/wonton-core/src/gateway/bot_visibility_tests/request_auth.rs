//! 请求头优先的认证来源和 origin 闸门。

use super::*;

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
