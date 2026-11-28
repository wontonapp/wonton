//! 角色：改名、刷新、新建、删除、换默认邀请角色，以及给好友换角色。

use super::*;

#[tokio::test]
async fn editing_a_role_keeps_the_page_in_step() {
    let rig = Rig::new().await;
    owner_ready(&rig).await;
    rig.app
        .dispatch(Action::Family {
            action: FamilyAction::UpdateRole {
                role_id: "child".to_owned(),
                draft: RoleDraft {
                    name: "小主人".to_owned(),
                    permissions: Default::default(),
                },
            },
        })
        .unwrap();
    family(&rig, "改名", |view| {
        view.roles
            .roles
            .iter()
            .any(|role| role.id == "child" && role.name == "小主人")
    })
    .await;
}

#[tokio::test]
async fn refreshing_roles_shows_a_role_added_elsewhere() {
    let rig = Rig::new().await;
    owner_ready(&rig).await;
    rig.core.family.add_role("guest", "客人");

    send(&rig, FamilyAction::RefreshRoles);
    let view = family(&rig, "刷新角色", |view| {
        view.roles.roles.iter().any(|role| role.id == "guest")
    })
    .await;
    assert!(!view.roles.loading);
    assert!(
        !rig.core.seen(&Method::GET, "/_wonton/api/roles").is_empty(),
        "刷新要真的去取角色"
    );
}

#[tokio::test]
async fn creating_a_role_puts_it_in_the_list() {
    let rig = Rig::new().await;
    owner_ready(&rig).await;
    let draft = RoleDraft {
        name: "客人".to_owned(),
        permissions: Default::default(),
    };

    send(&rig, FamilyAction::CreateRole { draft });
    family(&rig, "新角色", |view| {
        view.roles.roles.iter().any(|role| role.name == "客人")
    })
    .await;
    let body: RoleDraft = serde_json::from_str(
        &rig.core
            .seen(&Method::POST, "/_wonton/api/roles")
            .last()
            .expect("建角色要发给核心")
            .body,
    )
    .unwrap();
    assert_eq!(body.name, "客人");
}

#[tokio::test]
async fn deleting_a_role_drops_it_from_the_list() {
    let rig = Rig::new().await;
    owner_ready(&rig).await;

    send(
        &rig,
        FamilyAction::DeleteRole {
            role_id: "child".to_owned(),
        },
    );
    family(&rig, "删角色", |view| {
        !view.roles.roles.iter().any(|role| role.id == "child")
    })
    .await;
    assert!(
        !rig.core
            .seen(&Method::DELETE, "/_wonton/api/roles/child")
            .is_empty(),
        "删除要发给核心"
    );
}

#[tokio::test]
async fn setting_the_default_invite_role_applies_to_the_next_invite() {
    let rig = Rig::new().await;
    owner_ready(&rig).await;

    send(
        &rig,
        FamilyAction::SetDefaultRole {
            role_id: "friend".to_owned(),
        },
    );
    family(&rig, "换默认角色", |view| {
        view.roles.default_invite_role.as_deref() == Some("friend")
    })
    .await;

    invite(&rig, "Mia", None);
    family(&rig, "新人拿默认角色", |view| {
        view.members
            .members
            .iter()
            .any(|member| member.display_name == "Mia" && member.role_id == "friend")
    })
    .await;
}

#[tokio::test]
async fn assigning_a_friend_role_reports_success() {
    let rig = Rig::new().await;
    owner_ready(&rig).await;
    rig.core
        .answer_always_headered(Method::GET, "/_wonton/api/roles", storage_failure());
    refresh(&rig);
    family(&rig, "角色取失败", |view| view.roles.problem.is_some()).await;
    rig.core
        .stop_answering_headered(Method::GET, "/_wonton/api/roles");

    send(
        &rig,
        FamilyAction::AssignFriendRole {
            user_id: "@friend:test".to_owned(),
            role_id: "child".to_owned(),
        },
    );
    family(&rig, "好友换了角色", |view| {
        view.roles.problem.is_none()
    })
    .await;
    let seen = rig
        .core
        .seen(&Method::PUT, "/_wonton/api/friends/@friend:test/role");
    let body: serde_json::Value =
        serde_json::from_str(&seen.last().expect("换好友角色要发给核心").body).unwrap();
    assert_eq!(body["role_id"], "child");
}
