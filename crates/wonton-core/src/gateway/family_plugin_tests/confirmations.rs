//! 确认卡片的归属：放行时段只在主人的卡片上，家人停不了。

use hyper::{Method, StatusCode};
use iroh::SecretKey;

use crate::gateway::family_tests::send_as;
use crate::gateway::tests::{CORE, Harness};

use super::invite_record;

/// 放行时段只在主人的卡片上：家人停时段回 403 `not_yours`。家人列卡片、回答卡片见 `hop_limit_tests`。
#[tokio::test]
async fn members_cannot_stop_an_allow_window() {
    let h = Harness::start(None).await;
    let hers = SecretKey::generate().public();
    invite_record(&h, false).await;
    h.tunnel
        .devices
        .add_if(hers, None, Some("mia".to_owned()), |_| true)
        .unwrap();

    for (method, path, body) in [(Method::DELETE, "/_wonton/confirmations/some-id/window", "")] {
        let reply = send_as(
            &h.app,
            Some(hers),
            method,
            CORE,
            path,
            &[("content-type", "application/json")],
            body,
        )
        .await;
        assert_eq!(reply.status, StatusCode::FORBIDDEN, "{}", reply.text());
        assert_eq!(reply.error_kind(), Some("not_yours"), "{}", reply.text());
    }
}
