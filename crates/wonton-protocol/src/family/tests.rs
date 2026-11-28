use serde_json::json;

use super::*;
use crate::{AvatarImage, AvatarKind, BotChatUnavailable, Gate};

#[test]
fn conflicting_private_bot_room_facts_are_not_accepted() {
    let mut response = json!({
        "requester_user_id": "@alice:test",
        "result": {
            "state": "ready",
            "bot": {"plugin_id": "app.agent", "bot": "alice"},
            "user_id": "@alice-bot:test",
            "home_room": "!home:test",
            "room_problem": "invalid"
        }
    });
    assert!(serde_json::from_value::<OwnPrivateBotResponse>(response.clone()).is_err());

    response["result"]["room_problem"] = serde_json::Value::Null;
    let valid: OwnPrivateBotResponse = serde_json::from_value(response.clone()).unwrap();
    assert!(matches!(
        valid.result,
        OwnPrivateBotResult::Ready(PrivateBotIdentity {
            home_room: Some(_),
            room_problem: None,
            ..
        })
    ));

    response["result"]["home_room"] = serde_json::Value::Null;
    response["result"]["room_problem"] = json!("invalid");
    let invalid: OwnPrivateBotResponse = serde_json::from_value(response.clone()).unwrap();
    assert!(matches!(
        invalid.result,
        OwnPrivateBotResult::Ready(PrivateBotIdentity {
            home_room: None,
            room_problem: Some(PrivateBotRoomProblem::Invalid),
            ..
        })
    ));

    response["result"]["room_problem"] = serde_json::Value::Null;
    let unknown: OwnPrivateBotResponse = serde_json::from_value(response).unwrap();
    assert!(matches!(
        unknown.result,
        OwnPrivateBotResult::Ready(PrivateBotIdentity {
            home_room: None,
            room_problem: None,
            ..
        })
    ));
}

#[test]
fn plugin_identity_roles_round_trip() {
    for role in [Role::Owner, Role::Member, Role::Friend, Role::Other] {
        let value = serde_json::to_value(role).unwrap();
        assert_eq!(serde_json::from_value::<Role>(value).unwrap(), role);
    }
}

#[test]
fn me_carries_whether_it_is_the_owner_and_its_role() {
    let owner = json!({
        "user_id": "@owner:x.wonton.internal",
        "username": "owner",
        "is_owner": true,
        "role_id": null,
        "role_name": null,
        "permissions": null,
        "display_name": null,
        "email": null,
        "avatar": null,
    });
    let me: Me = serde_json::from_value(owner.clone()).unwrap();
    assert_eq!(serde_json::to_value(&me).unwrap(), owner);

    let member = json!({
        "user_id": "@alice:x.wonton.internal",
        "username": "alice",
        "is_owner": false,
        "role_id": "child",
        "role_name": "孩子",
        "permissions": {
            "open_apps": {"type": "selected", "plugin_ids": ["app.example.notes"]},
            "plugin_admin": {"type": "none"},
            "install_apps": "ask_admin",
            "model_settings": false,
            "server_settings": false,
            "manage_members": false,
            "add_friends": "ask_admin",
            "share_apps": false,
            "ai": "read_only",
            "privileged_tools": false,
            "private_bot": true,
            "shared_bots": false,
            "lend_own_bot": false,
        },
        "display_name": "爱丽丝",
        "email": null,
        "avatar": null,
    });
    let me: Me = serde_json::from_value(member.clone()).unwrap();
    assert_eq!(
        me.permissions.as_ref().unwrap().install_apps,
        Gate::AskAdmin
    );
    assert_eq!(serde_json::to_value(&me).unwrap(), member);
}

#[test]
fn values_a_newer_core_adds_fall_back_to_other() {
    let me: Me = serde_json::from_value(json!({
        "user_id": "@alice:x.wonton.internal",
        "username": "alice",
        "is_owner": false,
        "pronouns": "ignored",
    }))
    .unwrap();
    assert_eq!(me.role_id, None);
    assert_eq!(me.permissions, None);
    assert_eq!(
        serde_json::from_value::<MemberStatus>(json!("suspended")).unwrap(),
        MemberStatus::Other
    );
    assert_eq!(
        serde_json::from_value::<BotKeeper>(json!({"type": "team"})).unwrap(),
        BotKeeper::Other
    );
}

#[test]
fn an_me_without_an_avatar_is_fine() {
    let me: Me = serde_json::from_value(json!({
        "user_id": "@alice:x.wonton.internal",
        "username": "alice",
        "is_owner": false,
    }))
    .unwrap();
    assert_eq!(me.avatar, None);
}

#[test]
fn me_carries_the_avatar_as_a_media_handle() {
    let mut me = json!({
        "user_id": "@alice:x.wonton.internal",
        "username": "alice",
        "is_owner": false,
        "role_id": null,
        "role_name": null,
        "permissions": null,
        "display_name": null,
        "email": null,
        "avatar": {
            "media": {"token": r#"{"url":"mxc://x.wonton.internal/abc"}"#},
            "kind": "still",
            "width": null,
            "height": null,
            "poster": null,
        },
    });
    let parsed: Me = serde_json::from_value(me.clone()).unwrap();
    let avatar = parsed.avatar.as_ref().unwrap();
    assert_eq!(avatar.kind, AvatarKind::Still);
    assert_eq!(
        avatar.media.token,
        r#"{"url":"mxc://x.wonton.internal/abc"}"#
    );
    assert_eq!(avatar.width, None);
    assert_eq!(serde_json::to_value(&parsed).unwrap(), me);

    me["avatar"] = json!(null);
    let cleared: Me = serde_json::from_value(me).unwrap();
    assert_eq!(cleared.avatar, None);
}

#[test]
fn an_avatar_kind_the_core_does_not_know_shows_as_a_still_image() {
    let avatar = |kind: serde_json::Value| {
        serde_json::from_value::<AvatarImage>(json!({
            "media": {"token": r#"{"url":"mxc://x.wonton.internal/abc"}"#},
            "kind": kind,
            "width": null,
            "height": null,
            "poster": null,
        }))
    };
    assert_eq!(avatar(json!("still")).unwrap().kind, AvatarKind::Still);
    assert_eq!(avatar(json!("hologram")).unwrap().kind, AvatarKind::Still);
    let mut missing = json!({
        "media": {"token": r#"{"url":"mxc://x.wonton.internal/abc"}"#},
        "width": null,
        "height": null,
        "poster": null,
    });
    assert_eq!(
        serde_json::from_value::<AvatarImage>(missing.clone())
            .unwrap()
            .kind,
        AvatarKind::Still
    );
    missing["kind"] = json!("video");
    assert_eq!(
        serde_json::from_value::<AvatarImage>(missing).unwrap().kind,
        AvatarKind::Video
    );
}

#[test]
fn an_avatar_refusal_keeps_its_category() {
    let rejected = MyAvatarProblem::MediaRejected {
        problem: crate::AvatarMediaProblem::TooLarge,
    };
    assert_eq!(
        serde_json::to_value(&rejected).unwrap(),
        json!({"type": "media_rejected", "problem": "too_large"})
    );
    assert_eq!(
        serde_json::to_value(MyAvatarProblem::NotReady).unwrap(),
        json!({"type": "not_ready"})
    );
    assert_eq!(
        serde_json::to_value(MyAvatarProblem::NotLoggedIn {
            reason: "还没有登录 Matrix".to_owned()
        })
        .unwrap(),
        json!({"type": "not_logged_in", "reason": "还没有登录 Matrix"})
    );
    assert_eq!(
        serde_json::from_value::<MyAvatarProblem>(json!({"type": "quota"})).unwrap(),
        MyAvatarProblem::Other
    );
}

#[test]
fn keepers_name_the_person_or_the_household() {
    assert_eq!(
        serde_json::to_value(BotKeeper::User {
            username: "alice".into()
        })
        .unwrap(),
        json!({"type": "user", "username": "alice"})
    );
    assert_eq!(
        serde_json::to_value(BotKeeper::Household).unwrap(),
        json!({"type": "household"})
    );
}

#[test]
fn refusals_are_written_the_way_the_header_carries_them() {
    for refusal in [
        GatewayRefusal::ContactRequired,
        GatewayRefusal::PublicRoom,
        GatewayRefusal::HopLimit,
    ] {
        assert_eq!(
            serde_json::to_value(refusal).unwrap(),
            json!(refusal.as_str())
        );
    }
}

#[test]
fn bot_access_says_why_and_unknown_stays_forward_compatible() {
    assert_eq!(
        serde_json::to_value(BotAccess::Shared).unwrap(),
        json!({"state": "shared"})
    );
    assert_eq!(
        serde_json::to_value(BotAccess::Unusable {
            reason: BotChatUnavailable::AppNotShared
        })
        .unwrap(),
        json!({"state": "unusable", "reason": "app_not_shared"})
    );
    // 记录缺失走同一层 tag：`state` 不能丢，否则客户端分不出“用不了”和“不知道”。
    assert_eq!(
        serde_json::to_value(BotAccess::Unusable {
            reason: BotChatUnavailable::BotMissing
        })
        .unwrap(),
        json!({"state": "unusable", "reason": "bot_missing"})
    );
    assert_eq!(
        serde_json::from_value::<BotAccess>(json!({"state": "team"})).unwrap(),
        BotAccess::Unknown
    );
    assert_eq!(
        serde_json::from_value::<SharedBotsState>(json!("later")).unwrap(),
        SharedBotsState::Other
    );
    assert_eq!(
        serde_json::from_value::<BotChatUnavailable>(json!("owner_private")).unwrap(),
        BotChatUnavailable::OwnerPrivate
    );
    // 旧核心还会发旧值，别让它掉进 `Other`。
    assert_eq!(
        serde_json::from_value::<BotChatUnavailable>(json!("other_private_bot")).unwrap(),
        BotChatUnavailable::OwnerPrivate
    );
    assert_eq!(
        serde_json::to_value(BotChatUnavailable::OwnerPrivate).unwrap(),
        json!("owner_private")
    );
}

#[test]
fn refusal_errcodes_round_trip_and_ignore_foreign_codes() {
    for refusal in [
        GatewayRefusal::ContactRequired,
        GatewayRefusal::PublicRoom,
        GatewayRefusal::HopLimit,
        GatewayRefusal::ShareIdentity,
        GatewayRefusal::ShareRequired,
        GatewayRefusal::ShareRevoked,
        GatewayRefusal::ShareExpired,
        GatewayRefusal::Other,
    ] {
        assert_eq!(
            GatewayRefusal::from_errcode(refusal.errcode()),
            Some(refusal)
        );
        assert!(
            refusal
                .errcode()
                .starts_with(GATEWAY_REFUSAL_ERRCODE_PREFIX)
        );
    }
    assert_eq!(GatewayRefusal::from_errcode("M_FORBIDDEN"), None);
}

#[test]
fn an_invite_without_a_username_leaves_it_to_the_core() {
    let invite: MemberInvite =
        serde_json::from_value(json!({"display_name": "妈妈", "private_bot": true})).unwrap();
    assert_eq!(invite.username, None);
}
