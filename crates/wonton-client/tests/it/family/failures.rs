//! 刷新失败：服务器的错误按类别原样露出，失败不清掉已取到的列表。

use super::*;

#[tokio::test]
async fn a_server_failure_keeps_its_category() {
    let rig = Rig::new().await;
    owner_ready(&rig).await;
    rig.core.answer_always_headered(
        Method::GET,
        "/_wonton/api/members",
        crate::plugins::rig::Headered(
            500,
            Vec::new(),
            "{\"error\":{\"kind\":\"storage\",\"message\":\"盘满了\"}}".to_owned(),
        ),
    );
    refresh(&rig);
    let view = family(&rig, "存储失败", |view| view.members.problem.is_some()).await;
    let problem = view.members.problem.unwrap();
    assert_eq!(problem.reason, FamilyFailure::Storage);
    assert_eq!(problem.message, "盘满了");

    rig.core
        .stop_answering_headered(Method::GET, "/_wonton/api/members");
    refresh(&rig);
    family(&rig, "恢复", |view| view.members.problem.is_none()).await;
}

#[tokio::test]
async fn a_failed_member_refresh_keeps_the_last_list() {
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

    rig.core
        .answer_always_headered(Method::GET, "/_wonton/api/members", storage_failure());
    refresh(&rig);
    let view = family(&rig, "取成员失败", |view| {
        view.members.problem.is_some()
    })
    .await;
    assert_eq!(
        view.members.problem.as_ref().map(|problem| problem.reason),
        Some(FamilyFailure::Storage)
    );
    assert!(
        view.members
            .members
            .iter()
            .any(|member| member.username == "bob"),
        "失败不能把旧列表清空"
    );
    rig.core
        .stop_answering_headered(Method::GET, "/_wonton/api/members");
}

#[tokio::test]
async fn a_failed_request_refresh_keeps_the_last_list() {
    let rig = Rig::new().await;
    owner_ready(&rig).await;
    rig.core.family.join_request("r1", "pending", "child");
    refresh(&rig);
    family(&rig, "看到请求", |view| {
        !view.join_requests.requests.is_empty()
    })
    .await;

    rig.core.answer_always_headered(
        Method::GET,
        "/_wonton/api/members/join-requests",
        storage_failure(),
    );
    refresh(&rig);
    let view = family(&rig, "取请求失败", |view| {
        view.join_requests.problem.is_some()
    })
    .await;
    assert_eq!(
        view.join_requests
            .problem
            .as_ref()
            .map(|problem| problem.reason),
        Some(FamilyFailure::Storage)
    );
    assert!(
        view.join_requests
            .requests
            .iter()
            .any(|request| request.id == "r1"),
        "失败不能把旧请求清空"
    );
    rig.core
        .stop_answering_headered(Method::GET, "/_wonton/api/members/join-requests");
}
