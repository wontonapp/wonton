//! 设备列表与吊销：家人只看得到、吊销得了自己的设备。

use hyper::{Method, StatusCode};
use iroh::SecretKey;

use crate::gateway::tests::{CORE, Harness};

use super::support::*;

/// 家人的设备列表只看自己的，也吊销不了别人的。
#[tokio::test]
async fn a_member_sees_and_revokes_only_her_own_devices() {
    let h = Harness::start(None).await;
    let hers = SecretKey::generate().public();
    let bobs = SecretKey::generate().public();
    let owners = SecretKey::generate().public();
    seed_member(&h, "mia", false);
    seed_member(&h, "bob", false);
    h.tunnel
        .devices
        .add_if(hers, None, Some("mia".to_owned()), |_| true)
        .unwrap();
    h.tunnel
        .devices
        .add_if(bobs, None, Some("bob".to_owned()), |_| true)
        .unwrap();
    h.tunnel.devices.add(owners, None).unwrap();

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
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.text());
    let listed: Vec<crate::device::Device> =
        serde_json::from_value(reply.json()["devices"].clone()).unwrap();
    assert_eq!(
        listed.iter().map(|d| d.id).collect::<Vec<_>>(),
        [hers],
        "只看得到自己的"
    );

    // 吊销别人的设备回 403 not_yours；吊销自己的成功。
    for (id, want) in [
        (bobs, Some("not_yours")),
        (owners, Some("not_yours")),
        (hers, None),
    ] {
        let reply = send_as(
            &h.app,
            Some(hers),
            Method::DELETE,
            CORE,
            &format!("/_wonton/api/devices/{id}"),
            &[],
            "",
        )
        .await;
        match want {
            Some(kind) => {
                assert_eq!(reply.status, StatusCode::FORBIDDEN, "{}", reply.text());
                assert_eq!(reply.error_kind(), Some(kind));
            }
            None => assert_eq!(reply.status, StatusCode::OK, "{}", reply.text()),
        }
    }
    assert!(!h.tunnel.devices.contains(&hers), "自己的设备吊销掉了");
    assert!(h.tunnel.devices.contains(&owners), "别人的设备还在");
}
