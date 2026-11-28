//! 成员行：移除后从名单消失，改成员角色就地更新。

use super::*;

#[tokio::test]
async fn removal_drops_the_member_from_the_list() {
    let rig = Rig::new().await;
    owner_ready(&rig).await;
    rig.core.family.add_member("bob", "friend");
    refresh(&rig);
    family(&rig, "看到 bob", |view| {
        view.members
            .members
            .iter()
            .any(|member| member.username == "bob")
    })
    .await;

    rig.app
        .dispatch(Action::Family {
            action: FamilyAction::RemoveMember {
                username: "bob".to_owned(),
                delete_home: true,
                keep_bot_data: false,
            },
        })
        .unwrap();
    family(&rig, "移除后消失", |view| {
        !view
            .members
            .members
            .iter()
            .any(|member| member.username == "bob")
    })
    .await;
    assert_eq!(
        rig.core.family.member("bob").unwrap().status,
        MemberStatus::Removed
    );
    let seen = rig
        .core
        .seen(&Method::POST, "/_wonton/api/members/bob/remove");
    let body = &seen.last().expect("移除要发给核心").body;
    assert!(body.contains("\"delete_home\":true"));
}

#[tokio::test]
async fn assigning_a_member_role_updates_the_row() {
    let rig = Rig::new().await;
    owner_ready(&rig).await;
    rig.core.family.add_member("bob", "friend");
    refresh(&rig);
    family(&rig, "看到 bob", |view| {
        view.members
            .members
            .iter()
            .any(|member| member.username == "bob")
    })
    .await;

    send(
        &rig,
        FamilyAction::AssignMemberRole {
            username: "bob".to_owned(),
            role_id: "child".to_owned(),
        },
    );
    family(&rig, "换角色", |view| {
        view.members
            .members
            .iter()
            .any(|member| member.username == "bob" && member.role_id == "child")
    })
    .await;
    let seen = rig.core.seen(&Method::PUT, "/_wonton/api/members/bob/role");
    let body: serde_json::Value =
        serde_json::from_str(&seen.last().expect("换角色要发给核心").body).unwrap();
    assert_eq!(body["role_id"], "child");
}
