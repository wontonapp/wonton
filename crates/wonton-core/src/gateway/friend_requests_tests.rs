//! 外人发来的加好友请求（ADR-0088 第 98 行）：MSC4155 在 Tuwunel 就挡住了家外的邀请，到不了审批。
//! 公网落地页让外人留一条请求：目标角色还受限时开管理员卡片，批准把它们写进目标成员的 `allowed_users`；
//! 拒绝、过期、角色变了都如实回给发起方。落地页走真实的公网入口（server_name 的 `Host`，没有隧道身份）。

use axum::body::Body;
use hyper::{Method, StatusCode, header};
use serde_json::json;
use tower::ServiceExt;

use super::{OTHER, REMOTE, Rig, start, start_with};

const PAGE: &str = "/_wonton/friend-request";
const CAROL: &str = "@carol:elsewhere.test";

/// 记一个角色受限的家人，返回用户名。
fn restricted_member(rig: &Rig) -> &'static str {
    rig.members
        .invite(crate::members::MemberRecord::for_test("kid"))
        .unwrap();
    rig.members.set_role("kid", "child").unwrap();
    "kid"
}

/// 起一个挂住的请求，返回它的响应任务。
fn request(rig: &Rig, user: &str, to: &str) -> tokio::task::JoinHandle<(StatusCode, String)> {
    let entry = rig.entry.clone();
    let (user, to) = (user.to_owned(), to.to_owned());
    tokio::spawn(async move {
        let body = format!("user_id={}&to={}", encode(&user), encode(&to));
        let request = hyper::Request::builder()
            .method(Method::POST)
            .uri(PAGE)
            .header(header::HOST, crate::matrix::fake::SERVER)
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .body(Body::from(body))
            .unwrap();
        let response = entry.clone().oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::SEE_OTHER);
        // 交请求是 303 到这条请求的进度页；跟着走，和浏览器一样，直到页面不再自己刷新。
        let location = response.headers()[header::LOCATION]
            .to_str()
            .unwrap()
            .to_owned();
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            let request = hyper::Request::builder()
                .uri(&location)
                .header(header::HOST, crate::matrix::fake::SERVER)
                .body(Body::empty())
                .unwrap();
            let response = entry.clone().oneshot(request).await.unwrap();
            let status = response.status();
            let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .unwrap();
            let page = String::from_utf8_lossy(&bytes).into_owned();
            if !page.contains("http-equiv=\"refresh\"") {
                return (status, page);
            }
            assert!(tokio::time::Instant::now() < deadline, "进度页一直没答完");
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    })
}

async fn settle(waited: tokio::task::JoinHandle<(StatusCode, String)>) -> (StatusCode, String) {
    tokio::time::timeout(std::time::Duration::from_secs(5), waited)
        .await
        .expect("请求要有回应")
        .unwrap()
}

fn encode(value: &str) -> String {
    value
        .bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'*' => {
                (byte as char).to_string()
            }
            other => format!("%{other:02X}"),
        })
        .collect()
}

#[tokio::test]
async fn the_page_offers_a_form() {
    let rig = start().await;
    let (status, page) = rig.public(Method::GET, PAGE).await;
    assert_eq!(status, StatusCode::OK);
    assert!(page.contains("name=\"user_id\""), "{page}");
    assert!(page.contains("name=\"to\""), "{page}");
    assert!(
        page.contains("action=\"/_wonton/friend-request\""),
        "{page}"
    );
}

#[tokio::test]
async fn a_request_waiting_for_approval_is_recorded_for_that_person_only() {
    let rig = start().await;
    let target = restricted_member(&rig);
    let waiting = request(&rig, CAROL, &format!("@{target}:test"));

    let id = rig.wait_card().await;
    rig.answer(&id, "approve").await;
    let (status, page) = settle(waiting).await;
    assert_eq!(status, StatusCode::OK, "{page}");
    assert_eq!(
        rig.filter(&format!("@{target}:test")).unwrap()["allowed_users"],
        json!([CAROL])
    );
    assert!(
        rig.filter(&format!("@{target}:test")).unwrap()["allowed_users"]
            .as_array()
            .unwrap()
            .iter()
            .all(|user| user.as_str() != Some(REMOTE))
    );
}

#[tokio::test]
async fn a_rejected_request_reports_it_and_records_nothing() {
    let rig = start().await;
    let target = restricted_member(&rig);
    let waiting = request(&rig, CAROL, &format!("@{target}:test"));

    let id = rig.wait_card().await;
    rig.answer(&id, "reject").await;
    let (status, _) = settle(waiting).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(rig.filter(&format!("@{target}:test")), None);
}

#[tokio::test]
async fn an_unanswered_request_times_out_and_records_nothing() {
    let rig = start_with(std::time::Duration::from_millis(100)).await;
    let target = restricted_member(&rig);
    let (status, _) = settle(request(&rig, CAROL, &format!("@{target}:test"))).await;
    assert_eq!(status, StatusCode::REQUEST_TIMEOUT);
    assert_eq!(rig.filter(&format!("@{target}:test")), None);
}

#[tokio::test]
async fn the_same_pair_is_not_asked_twice() {
    let rig = start().await;
    let target = restricted_member(&rig);
    let to = format!("@{target}:test");
    let waiting = request(&rig, CAROL, &to);
    let _id = rig.wait_card().await;

    let (status, _) = rig.submit(&[("user_id", CAROL), ("to", &to)]).await;
    assert_eq!(status, StatusCode::CONFLICT);
    drop(waiting);
}

#[tokio::test]
async fn people_on_this_server_need_no_request() {
    let rig = start().await;
    restricted_member(&rig);
    let (status, _) = rig
        .submit(&[("user_id", "@owner:test"), ("to", "@kid:test")])
        .await;
    assert_eq!(status, StatusCode::CONFLICT);
}

#[tokio::test]
async fn an_open_role_needs_no_request() {
    let rig = start().await;
    rig.members
        .invite(crate::members::MemberRecord::for_test("mia"))
        .unwrap();
    rig.members.set_role("mia", "family").unwrap();
    let (status, _) = rig.submit(&[("user_id", CAROL), ("to", "@mia:test")]).await;
    assert_eq!(status, StatusCode::CONFLICT);
}

#[tokio::test]
async fn a_deny_role_cannot_use_the_page() {
    let rig = start().await;
    rig.members
        .invite(crate::members::MemberRecord::for_test("li"))
        .unwrap();
    rig.members
        .set_role("li", wonton_protocol::DEFAULT_INVITE_ROLE)
        .unwrap();
    let (status, page) = rig.submit(&[("user_id", CAROL), ("to", "@li:test")]).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(page.contains("cannot add friends"), "{page}");
}

#[tokio::test]
async fn an_unknown_target_is_not_found() {
    let rig = start().await;
    let (status, _) = rig
        .submit(&[("user_id", CAROL), ("to", "@nobody:test")])
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn an_invalid_requester_is_rejected_as_input() {
    let rig = start().await;
    restricted_member(&rig);
    let (status, _) = rig
        .submit(&[("user_id", "carol"), ("to", "@kid:test")])
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn a_target_losing_the_gate_while_waiting_is_reported() {
    let rig = start().await;
    let target = restricted_member(&rig);
    let waiting = request(&rig, CAROL, &format!("@{target}:test"));
    let id = rig.wait_card().await;
    rig.members.set_role("kid", "family").unwrap();
    rig.answer(&id, "approve").await;
    let (status, _) = settle(waiting).await;
    // 已经不用审批了：如实说清楚，名单不再写。
    assert_eq!(status, StatusCode::OK);
    assert!(
        rig.no_allowance("@kid:test"),
        "{:?}",
        rig.filter("@kid:test")
    );
}

#[tokio::test]
async fn a_removed_target_is_refused_on_approval() {
    let rig = start().await;
    let target = restricted_member(&rig);
    let waiting = request(&rig, CAROL, &format!("@{target}:test"));
    let id = rig.wait_card().await;
    rig.members.remove("kid", false, false).unwrap();
    rig.answer(&id, "approve").await;
    let (status, _) = settle(waiting).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn pending_requests_are_capped() {
    let rig = start().await;
    let target = restricted_member(&rig);
    let to = format!("@{target}:test");
    let mut waiting = Vec::new();
    for n in 0..3 {
        waiting.push(request(&rig, &format!("@user{n}:elsewhere.test"), &to));
    }
    rig.wait_cards(3).await;
    // 第 9 条直接 429。
    let (status, _) = rig.submit(&[("user_id", OTHER), ("to", &to)]).await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    drop(waiting);
}
