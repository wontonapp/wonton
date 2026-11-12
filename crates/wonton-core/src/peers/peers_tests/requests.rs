//! 配对请求的行为测试：收下和限频、拒绝和撤销、重名和用户 ID 的检查、互相发请求，以及取消请求。

use super::*;

/// 同一张码扫两次都收下：只有一条请求，第二次更新时间，不再提醒。
#[tokio::test]
async fn the_same_endpoint_sends_again_without_a_new_request_or_notice() {
    let a = TestCore::start("Alex", "a.example.internal").await;
    let b = TestCore::start("Alice", "b.example.internal").await;

    let code = a.peers.issue_code();
    b.scan_code(&code, &a).await.unwrap();
    let first = a.request_now("b.example.internal");
    b.set_owner_name("改名了");
    b.scan_code(&code, &a).await.unwrap();
    // 第二次扫码在回应之前就把那一条改好了，回来时已经是最新内容。
    let second = a.request_now("b.example.internal");
    assert_eq!(second.owner_display_name, "改名了");
    assert_eq!(second.id, first.id, "还是同一行");
    assert_eq!(a.peers.view(&langs()).requests.len(), 1);
    // 更新那一条不走提醒：第一条到了以后，条数不再涨。
    a.wait_notices(1).await;
    assert_eq!(a.notices().len(), 1, "再发不重复提醒");
}

/// 提醒按主人的语言写（ADR-0043）：日语的主人收到日语句子，链接照样带上。
#[tokio::test]
async fn the_notice_follows_the_owners_language() {
    let a = TestCore::start_in("ja", "Alex", "a.example.internal").await;
    let b = TestCore::start("Alice", "b.example.internal").await;

    let code = a.peers.issue_code();
    b.scan_code(&code, &a).await.unwrap();
    a.request_now("b.example.internal");
    a.wait_notices(1).await;
    let body = a.notices().remove(0)["body"].as_str().unwrap().to_owned();
    assert!(body.contains("のサーバー"), "{body}");
    assert!(body.contains("wonton://settings/peers"), "{body}");
}

/// 主人拒绝：请求进“已拒绝”，对方收到拒绝；再发直接回“对方拒绝了”且不再提醒。
/// 撤销拒绝后照常收下。
#[tokio::test]
async fn a_rejection_is_remembered_until_the_owner_unrejects() {
    let a = TestCore::start("Alex", "a.example.internal").await;
    let b = TestCore::start("Alice", "b.example.internal").await;

    let code = a.peers.issue_code();
    b.scan_code(&code, &a).await.unwrap();
    a.request_now("b.example.internal");
    a.reject("b.example.internal").await.unwrap();

    // 扫码方画出“对方拒绝了”，出码方留着被拒绝的那一条。
    b.wait_row_status("a.example.internal", PeerStatus::Declined)
        .await;
    let rejected = a.rejected("b.example.internal").expect("拒绝要记下来");
    assert!(a.peers.view(&langs()).requests.is_empty());

    // 再扫：回“对方拒绝了”，不新建请求、不再提醒。
    let error = b.scan_code(&code, &a).await.unwrap_err();
    assert_eq!(error, PeerError::PeerRejected);
    assert!(a.peers.view(&langs()).requests.is_empty());
    assert_eq!(a.notices().len(), 1, "被拒绝的再发不提醒");

    // 撤销拒绝：再扫照常收下，又来一条提醒。
    a.unreject("b.example.internal").await.unwrap();
    assert_eq!(a.rejected("b.example.internal"), None);
    assert_eq!(a.rejected("b.example.internal").map(|r| r.id), None);
    b.scan_code(&code, &a).await.unwrap();
    let request = a.request_now("b.example.internal");
    assert_ne!(request.id, rejected.id);
    a.wait_notices(2).await;
}

/// 拒绝、撤销和请求都落盘：重启后拒绝还在，请求还在，撤销也还在。
#[tokio::test]
async fn requests_and_rejections_survive_a_restart() {
    let mut a = TestCore::start("Alex", "a.example.internal").await;
    let b = TestCore::start("Alice", "b.example.internal").await;
    let c = TestCore::start("Chen", "c.example.internal").await;

    let code = a.peers.issue_code();
    b.scan_code(&code, &a).await.unwrap();
    a.request_now("b.example.internal");
    c.scan_code(&code, &a).await.unwrap();
    a.request_now("c.example.internal");
    a.reject("c.example.internal").await.unwrap();
    a.wait_notices(2).await;

    let (reopened, _) = a.reopen().await;
    let view = reopened.view(&langs());
    assert_eq!(view.requests.len(), 1, "待处理的请求还在");
    assert_eq!(view.requests[0].server_name, "b.example.internal");
    assert_eq!(view.rejected.len(), 1, "拒绝还在");
    assert_eq!(view.rejected[0].server_name, "c.example.internal");
}

/// 拒绝也经问结果送到：扫码方看到“对方拒绝了”。
#[tokio::test]
async fn a_rejection_is_picked_up_by_the_poll() {
    let a = TestCore::start("Alex", "a.example.internal").await;
    let b = TestCore::start("Alice", "b.example.internal").await;

    let code = a.peers.issue_code();
    b.scan_as(
        &code,
        &a,
        PeerIdentity {
            addrs: Vec::new(),
            ..b.peers.own_identity()
        },
    )
    .await
    .unwrap();
    a.request_now("b.example.internal");
    a.reject("b.example.internal").await.unwrap();

    b.poll("a.example.internal").await;
    b.wait_row_status("a.example.internal", PeerStatus::Declined)
        .await;
    assert!(b.peers.lookup("a.example.internal").is_none());
}

/// 用户 ID 不在自报的服务器名下：直接拒，不落请求。
#[tokio::test]
async fn a_user_id_outside_the_reported_server_name_is_refused() {
    let a = TestCore::start("Alex", "a.example.internal").await;
    let b = TestCore::start("Alice", "b.example.internal").await;

    let code = a.peers.issue_code();
    let error = b
        .scan_as(
            &code,
            &a,
            PeerIdentity {
                owner_user_id: "@alice:elsewhere.example".to_owned(),
                ..b.peers.own_identity()
            },
        )
        .await
        .unwrap_err();
    assert_eq!(error, PeerError::CodeInvalid);
    assert!(a.peers.view(&langs()).requests.is_empty());

    // 扫自己：不用连，直接说清。
    let own = a.peers.issue_code();
    let error = a.scan_code(&own, &a).await.unwrap_err();
    assert_eq!(error, PeerError::SelfPairing);
}

/// 待处理的请求满 20 条、或一小时新收满 10 条：回“对方暂时收不了请求”。
#[tokio::test]
async fn the_request_limits_refuse_with_too_many_requests() {
    let scanner = TestCore::start("Alice", "b.example.internal").await;

    // 20 条一直在等的请求（收到的时间很旧，不算进这一小时）。
    let a = TestCore::start_seeded("zh", "Alex", "a.example.internal", (0..20).collect(), 0).await;
    let error = scanner
        .scan_code(&a.peers.issue_code(), &a)
        .await
        .unwrap_err();
    assert_eq!(error, PeerError::TooManyRequests);

    // 一条待处理的都没有，但这一小时已经收过 10 条。
    let a = TestCore::start_seeded("zh", "Alex", "a.example.internal", Vec::new(), 10).await;
    let error = scanner
        .scan_code(&a.peers.issue_code(), &a)
        .await
        .unwrap_err();
    assert_eq!(error, PeerError::TooManyRequests);

    // 什么都没有：照常收下。
    let a = TestCore::start_seeded("zh", "Alex", "a.example.internal", Vec::new(), 0).await;
    scanner.scan_code(&a.peers.issue_code(), &a).await.unwrap();
    a.request_now("b.example.internal");
}

/// 取消自己发出的请求：对方的待处理请求跟着没。
#[tokio::test]
async fn cancelling_our_request_removes_it_on_the_other_side() {
    let a = TestCore::start("Alex", "a.example.internal").await;
    let b = TestCore::start("Alice", "b.example.internal").await;

    let code = a.peers.issue_code();
    b.scan_code(&code, &a).await.unwrap();
    a.request_now("b.example.internal");

    assert!(b.peers.remove("a.example.internal").await.unwrap());
    a.wait_for(
        |core| core.peers.view(&langs()).requests.is_empty(),
        "对方的请求消失",
    )
    .await;
    b.wait_row_gone("a.example.internal").await;
}

/// 同意一条已经不在的请求：`RequestNotFound`。
#[tokio::test]
async fn approving_a_gone_request_says_not_found() {
    let a = TestCore::start("Alex", "a.example.internal").await;
    let b = TestCore::start("Alice", "b.example.internal").await;
    let code = a.peers.issue_code();
    b.scan_code(&code, &a).await.unwrap();
    a.request_now("b.example.internal");
    a.approve("b.example.internal").await.unwrap();
    assert_eq!(
        a.peers.approve("gone").await.unwrap_err(),
        PeerError::RequestNotFound
    );
    assert_eq!(
        a.peers.reject("gone").await.unwrap_err(),
        PeerError::RequestNotFound
    );
    assert_eq!(
        a.peers.unreject("gone").await.unwrap_err(),
        PeerError::RequestNotFound
    );
}

/// 自报的显示名和已配对的一台重名、endpoint 不同：请求上标出和谁重名。
#[tokio::test]
async fn a_display_name_conflict_is_marked_on_the_request() {
    let a = TestCore::start("Alex", "a.example.internal").await;
    let b = TestCore::start("Alex", "b.example.internal").await;
    let c = TestCore::start("Chen", "c.example.internal").await;
    pair(&a, &b).await;

    let code = a.peers.issue_code();
    c.set_owner_name("Alex");
    c.scan_code(&code, &a).await.unwrap();
    let request = a.request_now("c.example.internal");
    assert_eq!(request.conflicts_with.as_deref(), Some("Alex"));
    assert!(request.detail.is_some());
    assert!(
        !request
            .detail
            .unwrap()
            .contains(&c.id_endpoint().to_string()),
        "详情只写前 8 位"
    );

    // 另一台不同名的请求不带冲突标记。
    let d = TestCore::start("Dee", "d.example.internal").await;
    d.scan_code(&code, &a).await.unwrap();
    assert_eq!(a.request_now("d.example.internal").conflicts_with, None);
}
/// 两边互相发了请求：直接成立，两边都不用再点。
#[tokio::test]
async fn mutual_requests_pair_without_an_approval() {
    let a = TestCore::start("Alex", "a.example.internal").await;
    let b = TestCore::start("Alice", "b.example.internal").await;

    let a_code = a.peers.issue_code();
    let b_code = b.peers.issue_code();
    b.scan_code(&a_code, &a).await.unwrap();
    a.request_now("b.example.internal");
    a.scan_code(&b_code, &b).await.unwrap();

    a.wait_paired("b.example.internal").await;
    b.wait_paired("a.example.internal").await;
    assert!(a.peers.view(&langs()).requests.is_empty());
    assert!(b.peers.view(&langs()).requests.is_empty());
}
