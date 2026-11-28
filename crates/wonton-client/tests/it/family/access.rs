//! 权限：没权限的人不摆操作，缺失项原样带回来，权限错误不被成员结果盖掉，自己被移除的读成移除。

use super::*;

#[tokio::test]
async fn a_member_without_the_permission_gets_no_actions_and_no_error_fuss() {
    let rig = Rig::new().await;
    rig.core.family.member_without_managing();
    rig.core.family.add_member("owner", "admin");
    rig.connected().await;
    refresh(&rig);
    let view = family(&rig, "权限读出来", |view| {
        !view.access.username.is_empty()
    })
    .await;
    assert!(!view.access.is_owner);
    assert!(!view.access.manages_members());
    assert!(!view.access.manages_roles());
    assert!(!view.access.chooses_role());
    assert_eq!(
        view.members.problem.as_ref().map(|p| p.reason),
        Some(FamilyFailure::PermissionRequired)
    );
    assert!(view.roles.problem.is_none(), "角色读不到不算失败");

    // 管理员能管成员但不能选角色：邀请只带名字，角色由核心给默认值。
    rig.core.family.manager();
    refresh(&rig);
    let view = family(&rig, "管理员", |view| view.access.manages_members()).await;
    assert!(!view.access.is_owner);
    assert!(!view.access.chooses_role());
    assert!(view.members.problem.is_none());

    invite(&rig, "来客", Some("child"));
    let view = family(&rig, "管理员邀请", |view| view.invite.is_some()).await;
    assert_eq!(view.invite.unwrap().display_name, "来客");
    let body: MemberInvite = serde_json::from_str(
        &rig.core
            .seen(&Method::POST, "/_wonton/api/members")
            .last()
            .unwrap()
            .body,
    )
    .unwrap();
    assert_eq!(body.role_id, None);
}

#[tokio::test]
async fn a_missing_permission_comes_back_with_the_item() {
    let rig = Rig::new().await;
    owner_ready(&rig).await;
    rig.core.family.fail(
        Method::POST,
        "/_wonton/api/members",
        403,
        Some("permission_required"),
        "{\"missing\":\"manage_members\"}",
    );
    invite(&rig, "米亚", None);
    let view = family(&rig, "权限失败", |view| view.members.problem.is_some()).await;
    let problem: &FamilyProblem = view.members.problem.as_ref().unwrap();
    assert_eq!(problem.reason, FamilyFailure::PermissionRequired);
    assert_eq_missing(problem, PermissionItem::ManageMembers);
}

fn assert_eq_missing(problem: &FamilyProblem, expected: PermissionItem) {
    assert_eq!(problem.missing, Some(expected));
}

#[tokio::test]
async fn an_access_failure_is_not_overwritten_by_a_member_result() {
    let rig = Rig::new().await;
    owner_ready(&rig).await;
    rig.core.family.add_member("bob", "friend");
    rig.core.answer_always_headered(
        Method::GET,
        "/_wonton/api/me",
        Headered(
            403,
            vec![(
                "x-wonton-error".to_owned(),
                "permission_required".to_owned(),
            )],
            "{\"missing\":\"manage_members\"}".to_owned(),
        ),
    );
    refresh(&rig);
    let view = family(&rig, "取自己失败", |view| {
        view.members.problem.is_some()
    })
    .await;
    assert_eq!(
        view.members.problem.as_ref().map(|problem| problem.reason),
        Some(FamilyFailure::PermissionRequired)
    );
    assert!(
        view.members
            .members
            .iter()
            .any(|member| member.username == "bob"),
        "成员列表照常取到"
    );
    rig.core
        .stop_answering_headered(Method::GET, "/_wonton/api/me");
}

#[tokio::test]
async fn a_removed_member_on_me_reads_as_removed() {
    let rig = Rig::new().await;
    owner_ready(&rig).await;
    rig.core.answer_always_headered(
        Method::GET,
        "/_wonton/api/me",
        Headered(
            403,
            vec![("x-wonton-error".to_owned(), "member_removed".to_owned())],
            "{\"error\":{\"kind\":\"member_removed\",\"message\":\"已经移除\"}}".to_owned(),
        ),
    );
    refresh(&rig);
    let view = family(&rig, "自己已被移除", |view| {
        view.members.problem.is_some()
    })
    .await;
    assert_eq!(
        view.members.problem.as_ref().map(|problem| problem.reason),
        Some(FamilyFailure::Removed)
    );
    rig.core
        .stop_answering_headered(Method::GET, "/_wonton/api/me");
}
