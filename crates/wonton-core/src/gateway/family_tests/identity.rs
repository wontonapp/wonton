//! 设备记的家人身份：记录里没有或者已经移除，就拒绝，不编一个身份放行（ADR-0045）。

use hyper::{Method, StatusCode};
use iroh::SecretKey;

use crate::gateway::tests::{CORE, Harness};
use crate::members::MemberRecord;

use super::support::*;

/// 设备记的家人不在记录里、或者已经移除：请求被拒，不编一个身份放行（失败即停，ADR-0045）。
#[tokio::test]
async fn a_device_whose_member_is_gone_is_rejected() {
    let h = Harness::start(None).await;

    // 设备登记的家人没有记录：当没认出人，401。
    let ghost = SecretKey::generate().public();
    h.tunnel
        .devices
        .add_if(ghost, None, Some("ghost".to_owned()), |_| true)
        .unwrap();
    let reply = send_as(&h.app, Some(ghost), Method::GET, CORE, ME, &[], "").await;
    assert_eq!(reply.status, StatusCode::UNAUTHORIZED, "{}", reply.text());

    // 家人被移除但设备登记还在（比如登记表恢复过旧备份）：一样拒。
    let hers = SecretKey::generate().public();
    h.members
        .invite(MemberRecord {
            display_name: "Mia".to_owned(),
            ..MemberRecord::for_test("mia")
        })
        .unwrap();
    h.tunnel
        .devices
        .add_if(hers, None, Some("mia".to_owned()), |_| true)
        .unwrap();
    // 直接走记录层移除：设备登记留着，正是要试的情形。
    h.members.remove("mia", false, false).unwrap();
    let reply = send_as(&h.app, Some(hers), Method::GET, CORE, ME, &[], "").await;
    assert_eq!(reply.status, StatusCode::UNAUTHORIZED, "{}", reply.text());

    // 走 plugin_denied 加 requester 的设备接口也一样：记录没了按认不出处理，回 401 不 panic。
    let hers = SecretKey::generate().public();
    h.tunnel
        .devices
        .add_if(hers, None, Some("mia".to_owned()), |_| true)
        .unwrap();
    let reply = send_as(
        &h.app,
        Some(hers),
        Method::GET,
        CORE,
        "/_wonton/api/devices",
        &[],
        "",
    )
    .await;
    assert_eq!(reply.status, StatusCode::UNAUTHORIZED, "{}", reply.text());
}
