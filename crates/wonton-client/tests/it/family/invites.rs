//! 邀请和码：邀请出码并把人带上名单，码过期能重发，关掉邀请只是关这一屏，作废才清掉。

use super::*;

#[tokio::test]
async fn an_invite_reports_a_code_and_puts_the_person_on_the_list_with_their_role() {
    let rig = Rig::new().await;
    owner_ready(&rig).await;

    invite(&rig, "米亚", Some("child"));
    let view = family(&rig, "出码", |view| view.invite.is_some()).await;
    let code = view.invite.unwrap();
    assert_eq!(code.display_name, "米亚");
    assert_eq!(
        rig.core.family.code(&code.username).as_deref(),
        Some(code.code.as_str())
    );

    let view = family(&rig, "成员带角色", |view| {
        view.members
            .members
            .iter()
            .any(|member| member.display_name == "米亚")
    })
    .await;
    let member = view
        .members
        .members
        .iter()
        .find(|member| member.display_name == "米亚")
        .unwrap();
    assert_eq!(member.role_id, "child");
    assert_eq!(member.role_name.as_deref(), Some("孩子"));
    assert_eq!(member.role_template, Some(RoleTemplate::Child));
    assert_eq!(member.status, MemberStatus::Invited);
    assert!(member.code.as_ref().is_some_and(|code| !code.expired));

    let body: MemberInvite = serde_json::from_str(
        &rig.core
            .seen(&Method::POST, "/_wonton/api/members")
            .last()
            .expect("邀请要发给核心")
            .body,
    )
    .unwrap();
    assert_eq!(body.role_id.as_deref(), Some("child"));
    assert!(!body.private_bot);
}

#[tokio::test]
async fn an_expired_code_is_marked_and_can_be_resent() {
    let rig = Rig::new().await;
    owner_ready(&rig).await;
    invite(&rig, "米亚", Some("child"));
    let Some(code) = family(&rig, "出码", |view| view.invite.is_some())
        .await
        .invite
    else {
        panic!("邀请要出码")
    };
    rig.core.family.expire_code(&code.username);
    refresh(&rig);
    let view = family(&rig, "标出过期", |view| {
        view.members
            .members
            .iter()
            .any(|member| member.display_name == "米亚")
    })
    .await;
    let member = view
        .members
        .members
        .iter()
        .find(|member| member.display_name == "米亚")
        .unwrap();
    assert!(member.code.as_ref().is_some_and(|code| code.expired));

    rig.app
        .dispatch(Action::Family {
            action: FamilyAction::ResendMemberCode {
                username: code.username.clone(),
            },
        })
        .unwrap();
    let resent = family(&rig, "重发", |view| {
        view.invite
            .as_ref()
            .is_some_and(|invite| invite.code != code.code)
    })
    .await;
    let fresh = resent.invite.unwrap();
    assert_eq!(
        rig.core.family.code(&code.username).as_deref(),
        Some(fresh.code.as_str())
    );
    let expires_at_ms = rig
        .core
        .family
        .member(&code.username)
        .unwrap()
        .pairing_code_expires_at_ms
        .unwrap();
    assert!(expires_at_ms > 0);
}

#[tokio::test]
async fn dismissing_an_invite_hides_the_code_and_keeps_it_on_the_server() {
    let rig = Rig::new().await;
    owner_ready(&rig).await;
    invite(&rig, "Mia", Some("child"));
    let view = family(&rig, "出码", |view| view.invite.is_some()).await;
    let username = view.invite.unwrap().username;
    assert!(rig.core.family.code(&username).is_some());

    send(&rig, FamilyAction::DismissMemberInvite);
    family(&rig, "关掉邀请页", |view| view.invite.is_none()).await;
    assert!(
        rig.core.family.code(&username).is_some(),
        "只是关掉这一屏，码还在服务器上"
    );
}

#[tokio::test]
async fn revoking_an_invite_code_clears_it_from_the_row() {
    let rig = Rig::new().await;
    owner_ready(&rig).await;
    invite(&rig, "Mia", Some("child"));
    let view = family(&rig, "出码", |view| view.invite.is_some()).await;
    let username = view.invite.unwrap().username;

    send(
        &rig,
        FamilyAction::RevokeMemberCode {
            username: username.clone(),
        },
    );
    family(&rig, "作废码", |view| {
        view.members
            .members
            .iter()
            .any(|member| member.username == username && member.code.is_none())
    })
    .await;
    assert_eq!(rig.core.family.code(&username), None);
    let path = format!("/_wonton/api/members/{username}/pairing-code");
    assert!(
        !rig.core.seen(&Method::DELETE, &path).is_empty(),
        "作废要发给核心"
    );
}
