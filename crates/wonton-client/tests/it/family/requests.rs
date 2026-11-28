//! 加入请求：请求带角色列出，批准把人改成已加入，拒绝作废码但留住邀请。

use super::*;

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
