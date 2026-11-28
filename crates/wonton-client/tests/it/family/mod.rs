//! 成员页（ADR-0088）的行为测试：只经导出给原生壳的接口驱动，隧道的对面是本机假扮的核心。
//!
//! 假核心按真核心的规矩记账：邀请记成员和码，批准把设备登记到人名下，拒绝连同码作废，移除改状态。
//! 断言看 ViewModel，也看假核心收到的请求。

mod failures;
mod invites;
mod roles;

use axum::http::Method;
use wonton_client::{Action, FamilyFailure, FamilyProblem, FamilyViewModel, MemberStatus};
use wonton_protocol::{FamilyAction, MemberInvite, PermissionItem, RoleDraft, RoleTemplate};

use crate::plugins::rig::{Headered, Rig, WAIT};

async fn family(
    rig: &Rig,
    what: &str,
    mut check: impl FnMut(&FamilyViewModel) -> bool,
) -> FamilyViewModel {
    let subscription = rig.app.subscribe_family();
    let waiting = async {
        while let Some(view) = subscription.next().await {
            if check(&view) {
                return view;
            }
        }
        panic!("订阅提前结束");
    };
    tokio::time::timeout(WAIT, waiting)
        .await
        .unwrap_or_else(|_| panic!("{what}：超时，当前：{:#?}", rig.app.family()))
}

fn refresh(rig: &Rig) {
    rig.app
        .dispatch(Action::Family {
            action: FamilyAction::RefreshFamily,
        })
        .unwrap();
}

fn send(rig: &Rig, action: FamilyAction) {
    rig.app.dispatch(Action::Family { action }).unwrap();
}

fn invite(rig: &Rig, name: &str, role: Option<&str>) {
    rig.app
        .dispatch(Action::Family {
            action: FamilyAction::InviteMember {
                display_name: name.to_owned(),
                username: None,
                private_bot: false,
                role_id: role.map(str::to_owned),
            },
        })
        .unwrap();
}

async fn owner_ready(rig: &Rig) {
    rig.core.family.owner();
    rig.connected().await;
    refresh(rig);
    family(rig, "成员页就绪", |view| {
        view.access.is_owner && !view.roles.roles.is_empty()
    })
    .await;
}

fn storage_failure() -> Headered {
    Headered(
        500,
        vec![("x-wonton-error".to_owned(), "storage".to_owned())],
        "{\"error\":{\"kind\":\"storage\",\"message\":\"盘满了\"}}".to_owned(),
    )
}

#[tokio::test]
async fn a_join_request_is_listed_with_its_role_and_approval_joins_the_person() {
    let rig = Rig::new().await;
    owner_ready(&rig).await;
    rig.core.family.add_member("pending", "child");
    rig.core.family.join_request("r1", "pending", "child");
    rig.core.family.add_member("other", "friend");

    refresh(&rig);
    let view = family(&rig, "请求", |view| {
        !view.join_requests.requests.is_empty()
    })
    .await;
    let request = &view.join_requests.requests[0];
    assert_eq!(request.display_name, "pending");
    assert_eq!(request.role_name.as_deref(), Some("孩子"));
    assert_eq!(request.device_name, "小米");

    let id = request.id.clone();
    rig.app
        .dispatch(Action::Family {
            action: FamilyAction::ApproveJoinRequest { id },
        })
        .unwrap();
    let view =
        family(&rig, "批准后", |view| {
            view.join_requests.requests.is_empty()
                && view.members.members.iter().any(|member| {
                    member.username == "pending" && member.status == MemberStatus::Joined
                })
        })
        .await;
    assert!(view.members.problem.is_none());
    assert_eq!(
        rig.core.family.member("pending").unwrap().status,
        MemberStatus::Joined
    );
}

#[tokio::test]
async fn rejecting_a_request_invalidates_the_code_and_keeps_the_invite() {
    let rig = Rig::new().await;
    owner_ready(&rig).await;
    invite(&rig, "米亚", Some("child"));
    let code = family(&rig, "出码", |view| view.invite.is_some())
        .await
        .invite
        .unwrap();
    rig.core.family.join_request("r1", &code.username, "child");
    let requests = family(&rig, "请求", |view| {
        !view.join_requests.requests.is_empty()
    })
    .await;
    let id = requests.join_requests.requests[0].id.clone();

    rig.app
        .dispatch(Action::Family {
            action: FamilyAction::RejectJoinRequest { id },
        })
        .unwrap();
    let view = family(&rig, "拒绝后", |view| {
        view.join_requests.requests.is_empty()
    })
    .await;
    assert!(view.members.problem.is_none());
    assert_eq_code_gone(&rig, &code.username);
    assert_eq!(
        rig.core.family.member(&code.username).unwrap().status,
        MemberStatus::Invited
    );
}

fn assert_eq_code_gone(rig: &Rig, username: &str) {
    assert_eq!(rig.core.family.code(username), None);
}

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
async fn leaving_the_page_stops_the_refresh() {
    let rig = Rig::new().await;
    owner_ready(&rig).await;
    rig.app
        .dispatch(Action::Family {
            action: FamilyAction::LeaveFamily,
        })
        .unwrap();
    // 让走之前已经发出、和定时器已经排上的一次取完。
    tokio::time::sleep(std::time::Duration::from_secs(6)).await;
    let before = rig.core.seen(&Method::GET, "/_wonton/api/members").len();
    tokio::time::sleep(std::time::Duration::from_secs(6)).await;
    let after = rig.core.seen(&Method::GET, "/_wonton/api/members").len();
    assert_eq!(before, after, "离开页面后不该再取列表");
}

/// 离开页面再回来：轮询要跟着恢复，不能只靠手动刷新那一次。
#[tokio::test]
async fn coming_back_to_the_page_resumes_the_polling() {
    let rig = Rig::new().await;
    owner_ready(&rig).await;
    rig.app
        .dispatch(Action::Family {
            action: FamilyAction::LeaveFamily,
        })
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_secs(6)).await;
    refresh(&rig);
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    let before = rig.core.seen(&Method::GET, "/_wonton/api/members").len();
    tokio::time::sleep(std::time::Duration::from_secs(6)).await;
    let after = rig.core.seen(&Method::GET, "/_wonton/api/members").len();
    assert!(after > before, "回到页面后定时重取要恢复");
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

#[tokio::test]
async fn refreshing_join_requests_shows_a_new_request() {
    let rig = Rig::new().await;
    owner_ready(&rig).await;
    rig.core.family.join_request("r1", "pending", "child");

    send(&rig, FamilyAction::RefreshJoinRequests);
    family(&rig, "刷新请求", |view| {
        view.join_requests
            .requests
            .iter()
            .any(|request| request.id == "r1")
    })
    .await;
    assert!(
        !rig.core
            .seen(&Method::GET, "/_wonton/api/members/join-requests")
            .is_empty(),
        "刷新要真的去取请求"
    );
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
