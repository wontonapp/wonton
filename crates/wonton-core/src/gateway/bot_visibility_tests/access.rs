//! 按归属、角色、插件开放范围判 BotAccess。

use super::*;

fn access(fixture: &Fixture, peer: TunnelPeer, reference: &BotRef) -> BotAccess {
    bot_access(&fixture.gateway, Some(peer), &HeaderMap::new(), reference).unwrap()
}

#[tokio::test]
async fn app_scope_and_shared_bots_gate_access_before_contacts() {
    let fixture = Fixture::new().await;
    let alice = fixture.member("alice", "@alice:test");
    let reference = bot("shared");
    fixture.seed(&[(
        "shared",
        "@shared:test",
        BotKeeper::Household,
        "!shared-home:test",
    )]);
    fixture.gateway.members.set_role("alice", "family").unwrap();

    let family = fixture.gateway.roles.get("family").unwrap();
    let mut permissions = family.permissions.clone();
    permissions.open_apps = AppScope::Selected {
        plugin_ids: vec!["app.other".into()],
    };
    fixture
        .gateway
        .roles
        .update(
            "family",
            &RoleDraft {
                name: "family".into(),
                permissions,
            },
        )
        .unwrap();
    assert_eq!(
        access(&fixture, alice, &reference),
        BotAccess::Unusable {
            reason: BotChatUnavailable::AppNotShared
        }
    );

    // 插件开了、角色能听共享 Bot：共享。
    fixture.gateway.members.set_role("alice", "admin").unwrap();
    assert_eq!(access(&fixture, alice, &reference), BotAccess::Shared);
}

#[tokio::test]
async fn own_bot_and_active_contact_do_not_depend_on_open_apps() {
    let mut fixture = Fixture::new().await;
    let alice = fixture.member("alice", "@alice:test");
    let own = bot("alice-bot");
    let contacted = bot("bob-bot");
    fixture.seed(&[
        (
            "alice-bot",
            "@alice-bot:test",
            BotKeeper::User {
                username: "alice".into(),
            },
            "!own-home:test",
        ),
        (
            "bob-bot",
            "@bob-bot:test",
            BotKeeper::User {
                username: "bob".into(),
            },
            "!bob-home:test",
        ),
    ]);
    fixture.gateway.members.set_role("alice", "family").unwrap();
    // 插件没开给她的角色：自己的 Bot 和已成立联系人照样能用。
    let family = fixture.gateway.roles.get("family").unwrap();
    let mut permissions = family.permissions.clone();
    permissions.open_apps = AppScope::Selected {
        plugin_ids: vec!["app.other".into()],
    };
    fixture
        .gateway
        .roles
        .update(
            "family",
            &RoleDraft {
                name: "family".into(),
                permissions,
            },
        )
        .unwrap();
    assert_eq!(access(&fixture, alice, &own), BotAccess::Own);
    fixture.contacts(contact_rows(
        &contacted,
        json!({"@alice:test": {"kind": "person", "state": "active", "updated_at_ms": 1}}),
    ));
    assert_eq!(access(&fixture, alice, &contacted), BotAccess::Contact);
}

#[tokio::test]
async fn a_missing_record_is_bot_missing_even_without_a_known_caller() {
    let fixture = Fixture::new().await;
    let facts = fixture.facts(None, &bot("missing"));
    assert!(facts.bot.is_none());
    assert_eq!(
        fixture.facts(None, &bot("missing")).caller,
        CallerFacts::Unknown
    );
    assert_eq!(
        access_of(&fixture.gateway, &bot("missing"), &facts),
        BotAccess::Unusable {
            reason: BotChatUnavailable::BotMissing
        }
    );
}

#[tokio::test]
async fn household_bot_without_shared_bots_says_role_lacks_it() {
    let fixture = Fixture::new().await;
    let alice = fixture.member("alice", "@alice:test");
    let reference = bot("shared");
    fixture.seed(&[(
        "shared",
        "@shared:test",
        BotKeeper::Household,
        "!shared-home:test",
    )]);
    assert_eq!(
        access(&fixture, alice, &reference),
        BotAccess::Unusable {
            reason: BotChatUnavailable::RoleLacksSharedBots
        }
    );
}
