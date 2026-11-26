use axum::body::Body;
use hyper::{Method, StatusCode, header};
use tower::ServiceExt;

use super::{KID_TOKEN, MATRIX_HOST, REMOTE, create_room, deny_role, start};

#[tokio::test]
async fn downgrade_while_waiting_for_approval_does_not_send_the_invite() {
    let rig = start().await;
    let device = rig.member_device("kid");
    rig.members.set_role("kid", "child").unwrap();
    let baseline = rig.fake.state().rooms.len();

    let app = rig.app.clone();
    let waiting = tokio::spawn(async move {
        let mut request = hyper::Request::builder()
            .method(Method::POST)
            .uri("/_matrix/client/v3/createRoom")
            .header(header::HOST, MATRIX_HOST)
            .header(header::AUTHORIZATION, format!("Bearer {KID_TOKEN}"))
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(create_room(&[REMOTE]).to_string()))
            .unwrap();
        request
            .extensions_mut()
            .insert(super::super::TunnelPeer(device));
        app.oneshot(request).await.unwrap().status()
    });

    let id = rig.wait_card().await;
    let role = rig.roles.create(&deny_role(&rig)).unwrap();
    rig.members.set_role("kid", &role.id).unwrap();
    rig.answer(&id, "approve").await;

    let status = waiting.await.unwrap();
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(rig.fake.state().rooms.len(), baseline);
}

#[tokio::test]
async fn revoked_device_cannot_send_an_invite_after_approval() {
    let rig = start().await;
    let device = rig.member_device("kid");
    rig.members.set_role("kid", "child").unwrap();
    let baseline = rig.fake.state().rooms.len();

    let app = rig.app.clone();
    let waiting = tokio::spawn(async move {
        let mut request = hyper::Request::builder()
            .method(Method::POST)
            .uri("/_matrix/client/v3/createRoom")
            .header(header::HOST, MATRIX_HOST)
            .header(header::AUTHORIZATION, format!("Bearer {KID_TOKEN}"))
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(create_room(&[REMOTE]).to_string()))
            .unwrap();
        request
            .extensions_mut()
            .insert(super::super::TunnelPeer(device));
        app.oneshot(request).await.unwrap().status()
    });

    let id = rig.wait_card().await;
    assert!(rig.tunnel.devices.remove(&device).unwrap());
    rig.answer(&id, "approve").await;

    let status = waiting.await.unwrap();
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(rig.fake.state().rooms.len(), baseline);
}
