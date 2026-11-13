use serde_json::json;

use super::*;

fn friend_template() -> Permissions {
    Permissions {
        open_apps: AppScope::Selected {
            plugin_ids: vec!["app.snake".into()],
        },
        plugin_admin: AdminScope::NoApps,
        install_apps: Gate::Deny,
        model_settings: false,
        server_settings: false,
        manage_members: false,
        add_friends: Gate::Deny,
        share_apps: false,
        ai: AiLevel::ReadOnly,
        privileged_tools: false,
        private_bot: false,
        shared_bots: false,
        lend_own_bot: false,
    }
}

#[test]
fn permissions_round_trip_with_their_wire_names() {
    let value = serde_json::to_value(friend_template()).unwrap();
    assert_eq!(
        value["open_apps"],
        json!({"type": "selected", "plugin_ids": ["app.snake"]})
    );
    assert_eq!(value["plugin_admin"], json!({"type": "none"}));
    assert_eq!(value["install_apps"], json!("deny"));
    assert_eq!(value["ai"], json!("read_only"));
    assert_eq!(
        serde_json::from_value::<Permissions>(value).unwrap(),
        friend_template()
    );
}

#[test]
fn values_a_newer_core_adds_fall_back_to_other() {
    let mut value = serde_json::to_value(friend_template()).unwrap();
    value["open_apps"] = json!({"type": "by_tag", "tag": "games"});
    value["plugin_admin"] = json!({"type": "by_tag"});
    value["install_apps"] = json!("ask_parent");
    value["ai"] = json!("everything");
    value["watch_tv"] = json!(true);
    let parsed: Permissions = serde_json::from_value(value).unwrap();
    assert_eq!(parsed.open_apps, AppScope::Other);
    assert_eq!(parsed.plugin_admin, AdminScope::Other);
    assert_eq!(parsed.install_apps, Gate::Other);
    assert_eq!(parsed.ai, AiLevel::Other);
    assert_eq!(
        serde_json::from_value::<RoleTemplate>(json!("guest")).unwrap(),
        RoleTemplate::Other
    );
    assert_eq!(
        serde_json::from_value::<PermissionItem>(json!("watch_tv")).unwrap(),
        PermissionItem::Other
    );
    assert_eq!(
        serde_json::from_value::<JoinOutcome>(json!("expired")).unwrap(),
        JoinOutcome::Other
    );
}

#[test]
fn plugins_see_capabilities_written_the_way_groups_and_headers_carry_them() {
    let capability = Capability::PluginAdmin;
    assert_eq!(
        serde_json::to_value(capability).unwrap(),
        json!(capability.as_str())
    );
    for level in [
        AiLevel::ReadOnly,
        AiLevel::AdminApprovesDeletes,
        AiLevel::SelfConfirms,
    ] {
        assert_eq!(serde_json::to_value(level).unwrap(), json!(level.as_str()));
    }
}

#[test]
fn a_refusal_names_the_missing_item() {
    let body: PermissionRequired =
        serde_json::from_value(json!({"missing": "install_apps"})).unwrap();
    assert_eq!(body.missing, PermissionItem::InstallApps);
}

#[test]
fn role_and_request_paths_nest_under_their_owners() {
    assert_eq!(member_role_path("alice"), "/_wonton/api/members/alice/role");
    assert_eq!(
        join_request_approve_path("r1"),
        "/_wonton/api/members/join-requests/r1/approve"
    );
    assert_eq!(
        plugin_audience_path("app.snake"),
        "/_wonton/api/plugins/app.snake/audience"
    );
}

/// 主人的权限在每一项上都不少于任何角色；最严的取值在每一项上不多于朋友模板。
#[test]
fn the_owner_has_more_of_everything_and_the_strictest_never_more_than_the_friend_template() {
    let rank = |gate: Gate| match gate {
        Gate::Deny | Gate::Other => 0,
        Gate::AskAdmin => 1,
        Gate::Allow => 2,
    };
    let level = |ai: AiLevel| match ai {
        AiLevel::ReadOnly | AiLevel::Other => 0,
        AiLevel::AdminApprovesDeletes => 1,
        AiLevel::SelfConfirms => 2,
    };
    let friend = friend_template();
    let owner = Permissions::owner();
    let strictest = Permissions::strictest();
    for (owner, friend, strictest) in [(&owner, &friend, &strictest)] {
        assert!(
            owner.install_apps == Gate::Allow
                && rank(owner.install_apps) > rank(friend.install_apps)
        );
        assert!(rank(strictest.install_apps) <= rank(friend.install_apps));
        assert!(
            owner.add_friends == Gate::Allow && rank(owner.add_friends) > rank(friend.add_friends)
        );
        assert!(rank(strictest.add_friends) <= rank(friend.add_friends));
        assert!(level(owner.ai) > level(friend.ai) && level(strictest.ai) <= level(friend.ai));
        for (owner, friend) in [
            (owner.model_settings, friend.model_settings),
            (owner.server_settings, friend.server_settings),
            (owner.manage_members, friend.manage_members),
            (owner.share_apps, friend.share_apps),
            (owner.privileged_tools, friend.privileged_tools),
            (owner.private_bot, friend.private_bot),
            (owner.shared_bots, friend.shared_bots),
            (owner.lend_own_bot, friend.lend_own_bot),
        ] {
            assert!(owner || !friend, "主人缺了一项朋友有的权限");
        }
        assert!(owner.open_apps == AppScope::All);
        assert!(strictest.open_apps != AppScope::All);
        assert!(owner.plugin_admin == AdminScope::AllOpenable);
        assert!(strictest.plugin_admin != AdminScope::AllOpenable);
    }
}

/// 建角色时没写的权限项按最严的算：旧的 App 不认识新项，发出来的草稿不能因此放开。
#[test]
fn a_draft_without_permission_fields_defaults_to_the_strictest() {
    let draft: RoleDraft = serde_json::from_value(json!({"name": "客人"})).unwrap();
    assert_eq!(draft.permissions, Permissions::strictest());
}
