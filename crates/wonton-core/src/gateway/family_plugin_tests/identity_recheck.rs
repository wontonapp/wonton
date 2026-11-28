//! 列表和详情在装载（慢的升级检查、Docker 请求）之后重新核对身份和角色。

use hyper::{Method, StatusCode};
use iroh::SecretKey;

use crate::gateway::family_tests::send_as;
use crate::gateway::tests::{CORE, Harness, Reply};
use crate::gateway::{TunnelPeer, api};

use super::invite_record;

#[tokio::test]
async fn plugin_list_rejects_a_revoked_or_reassigned_device() {
    for reassign_to_owner in [false, true] {
        let h = Harness::start(None).await;
        invite_record(&h, true).await;
        let device = SecretKey::generate().public();
        h.tunnel
            .devices
            .add_if(device, None, Some("mia".into()), |_| true)
            .unwrap();

        let mut request = Box::pin(api::list(
            axum::extract::State(h.gateway.clone()),
            Some(axum::extract::Extension(TunnelPeer(device))),
            hyper::HeaderMap::new(),
        ));
        // installed_checked 等待新启动的升级检查；单线程首轮不会运行该任务。
        assert!(futures_util::poll!(&mut request).is_pending());
        assert!(h.tunnel.devices.remove(&device).unwrap());
        if reassign_to_owner {
            h.tunnel.devices.add(device, None).unwrap();
        }
        let reply = Reply::of(request.await).await;
        assert_eq!(reply.status, StatusCode::UNAUTHORIZED, "{}", reply.text());
        assert_eq!(reply.error_kind(), Some("unauthenticated"));
    }
}

#[tokio::test]
async fn plugin_list_applies_the_current_role_after_loading() {
    let h = Harness::start(None).await;
    invite_record(&h, true).await;
    let device = SecretKey::generate().public();
    h.tunnel
        .devices
        .add_if(device, None, Some("mia".into()), |_| true)
        .unwrap();
    let mut request = Box::pin(api::list(
        axum::extract::State(h.gateway.clone()),
        Some(axum::extract::Extension(TunnelPeer(device))),
        hyper::HeaderMap::new(),
    ));
    assert!(futures_util::poll!(&mut request).is_pending());
    h.members.set_role("mia", "friend").unwrap();
    let reply = Reply::of(request.await).await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.text());
    assert!(
        reply.json()["plugins"]
            .as_array()
            .unwrap()
            .iter()
            .all(|plugin| plugin["id"] != "app.whoami")
    );
}

#[tokio::test]
async fn plugin_detail_rechecks_identity_and_role_after_loading() {
    for revoke_device in [true, false] {
        let h = Harness::start(None).await;
        invite_record(&h, true).await;
        let device = SecretKey::generate().public();
        h.tunnel
            .devices
            .add_if(device, None, Some("mia".into()), |_| true)
            .unwrap();
        let hold = h.docker.hold("GET /info");
        let app = h.app.clone();
        let waiting = tokio::spawn(async move {
            send_as(
                &app,
                Some(device),
                Method::GET,
                CORE,
                "/_wonton/api/plugins/app.whoami",
                &[],
                "",
            )
            .await
        });
        tokio::time::timeout(std::time::Duration::from_secs(5), hold.reached())
            .await
            .expect("detail request reached Docker");
        let (status, kind) = if revoke_device {
            assert!(h.tunnel.devices.remove(&device).unwrap());
            (StatusCode::UNAUTHORIZED, "unauthenticated")
        } else {
            h.members.set_role("mia", "friend").unwrap();
            (StatusCode::FORBIDDEN, "not_shared")
        };
        hold.release();
        let reply = tokio::time::timeout(std::time::Duration::from_secs(10), waiting)
            .await
            .expect("detail response")
            .unwrap();
        assert_eq!(reply.status, status, "{}", reply.text());
        assert_eq!(reply.error_kind(), Some(kind));
    }
}
