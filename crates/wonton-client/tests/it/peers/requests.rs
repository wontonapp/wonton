//! 请求的行为测试：同意、拒绝、撤销，以及失败按类别进状态。

use super::*;

/// 发来的请求列出来；同意后调接口并刷新，那条请求没了、多一台服务器。
#[tokio::test]
async fn approving_a_request_calls_the_api_and_refreshes() {
    let rig = Rig::new().await;
    rig.core.answer_always(
        Method::GET,
        PEERS_PATH,
        Canned(
            StatusCode::OK.as_u16(),
            view_with(
                Vec::new(),
                vec![request("r1", "abc12345.wonton")],
                Vec::new(),
            ),
        ),
    );
    rig.connected().await;
    wait_peers(&rig, |view| !view.requests.is_empty()).await;
    assert_eq!(rig.app.peers().requests[0].conflicts_with, None);

    rig.core.answer(
        Method::POST,
        &peer_request_approve_path("r1"),
        Canned(StatusCode::OK.as_u16(), "{}".into()),
    );
    rig.core.answer_always(
        Method::GET,
        PEERS_PATH,
        Canned(
            StatusCode::OK.as_u16(),
            peers_view(vec![row("abc12345.wonton")]),
        ),
    );
    rig.app
        .dispatch(Action::Peers {
            action: wonton_protocol::PeerAction::ApprovePeerRequest { id: "r1".into() },
        })
        .unwrap();
    wait_peers(&rig, |view| {
        view.state == PeersState::Ready && view.requests.is_empty() && view.peers.len() == 1
    })
    .await;
    assert_eq!(
        rig.core
            .seen(&Method::POST, &peer_request_approve_path("r1"))
            .len(),
        1
    );
}

/// 拒绝进“已拒绝”；撤销后那条没了。
#[tokio::test]
async fn rejecting_and_unrejecting_requests_call_the_api() {
    let rig = Rig::new().await;
    rig.core.answer_always(
        Method::GET,
        PEERS_PATH,
        Canned(
            StatusCode::OK.as_u16(),
            view_with(
                Vec::new(),
                vec![request("r1", "abc12345.wonton")],
                Vec::new(),
            ),
        ),
    );
    rig.connected().await;
    wait_peers(&rig, |view| !view.requests.is_empty()).await;

    rig.core.answer(
        Method::POST,
        &peer_request_reject_path("r1"),
        Canned(StatusCode::OK.as_u16(), "{}".into()),
    );
    rig.core.answer_always(
        Method::GET,
        PEERS_PATH,
        Canned(
            StatusCode::OK.as_u16(),
            view_with(
                Vec::new(),
                Vec::new(),
                vec![rejected("r1", "abc12345.wonton")],
            ),
        ),
    );
    rig.app
        .dispatch(Action::Peers {
            action: wonton_protocol::PeerAction::RejectPeerRequest { id: "r1".into() },
        })
        .unwrap();
    wait_peers(&rig, |view| !view.rejected.is_empty()).await;

    rig.core.answer(
        Method::DELETE,
        &peer_rejected_path("r1"),
        Canned(StatusCode::OK.as_u16(), "{}".into()),
    );
    rig.core.answer_always(
        Method::GET,
        PEERS_PATH,
        Canned(StatusCode::OK.as_u16(), peers_view(Vec::new())),
    );
    rig.app
        .dispatch(Action::Peers {
            action: wonton_protocol::PeerAction::UnrejectPeerRequest { id: "r1".into() },
        })
        .unwrap();
    wait_peers(&rig, |view| {
        view.state == PeersState::Ready && view.rejected.is_empty()
    })
    .await;
    assert_eq!(
        rig.core
            .seen(&Method::POST, &peer_request_reject_path("r1"))
            .len(),
        1
    );
    assert_eq!(
        rig.core
            .seen(&Method::DELETE, &peer_rejected_path("r1"))
            .len(),
        1
    );
}

/// 处理请求失败：家里的服务器给的类别落进 `Failed`。
#[tokio::test]
async fn a_failed_request_action_fails_with_the_server_s_category() {
    let rig = Rig::new().await;
    rig.core.answer_always(
        Method::GET,
        PEERS_PATH,
        Canned(
            StatusCode::OK.as_u16(),
            view_with(
                Vec::new(),
                vec![request("r1", "abc12345.wonton")],
                Vec::new(),
            ),
        ),
    );
    rig.connected().await;
    wait_peers(&rig, |view| !view.requests.is_empty()).await;
    rig.core.answer_headered(
        Method::POST,
        &peer_request_approve_path("r1"),
        peer_error_headered(404, "request_not_found"),
    );
    rig.app
        .dispatch(Action::Peers {
            action: wonton_protocol::PeerAction::ApprovePeerRequest { id: "r1".into() },
        })
        .unwrap();
    assert!(matches!(
        wait_failed(&rig, PeerError::RequestNotFound).await.state,
        PeersState::Failed {
            error: PeerError::RequestNotFound,
            ..
        }
    ));
}
