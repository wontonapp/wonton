//! 出码页的行为测试：取码、关掉时清掉、重置。

use super::*;

/// 打开出码页取码；刷新不清掉码，关掉页面才清。
#[tokio::test]
async fn showing_the_code_gets_it_and_hiding_clears_it() {
    let rig = Rig::new().await;
    connected_and_listed(&rig, Vec::new()).await;

    let code = PeerCode {
        code: SecretString::new("wonton://peer?v=1&token=abc"),
    };
    rig.core.answer(
        Method::GET,
        &peer_code_path(),
        Canned(
            StatusCode::OK.as_u16(),
            serde_json::to_string(&code).unwrap(),
        ),
    );
    rig.app
        .dispatch(Action::Peers {
            action: wonton_protocol::PeerAction::ShowPeerCode,
        })
        .unwrap();
    let view = wait_peers(&rig, |view| view.code.is_some()).await;
    assert_eq!(
        view.code.unwrap().code.expose(),
        "wonton://peer?v=1&token=abc"
    );
    assert_eq!(rig.core.seen(&Method::GET, &peer_code_path()).len(), 1);

    // 列表刷新不带码：码留着。
    rig.app
        .dispatch(Action::Peers {
            action: wonton_protocol::PeerAction::RefreshPeers,
        })
        .unwrap();
    wait_peers(&rig, |view| view.state == PeersState::Ready).await;
    assert!(rig.app.peers().code.is_some(), "刷新不该把码清掉");

    // 关掉出码页：码清掉，不再打接口。
    let before = rig.core.seen(&Method::GET, &peer_code_path()).len();
    rig.app
        .dispatch(Action::Peers {
            action: wonton_protocol::PeerAction::HidePeerCode,
        })
        .unwrap();
    wait_peers(&rig, |view| view.code.is_none()).await;
    assert_eq!(rig.core.seen(&Method::GET, &peer_code_path()).len(), before);
}

/// 重置码：新码进 ViewModel。
#[tokio::test]
async fn resetting_the_code_shows_the_new_one() {
    let rig = Rig::new().await;
    connected_and_listed(&rig, Vec::new()).await;

    let first = PeerCode {
        code: SecretString::new("wonton://peer?v=1&token=one"),
    };
    let second = PeerCode {
        code: SecretString::new("wonton://peer?v=1&token=two"),
    };
    rig.core.answer(
        Method::GET,
        &peer_code_path(),
        Canned(
            StatusCode::OK.as_u16(),
            serde_json::to_string(&first).unwrap(),
        ),
    );
    rig.core.answer(
        Method::POST,
        &peer_code_reset_path(),
        Canned(
            StatusCode::OK.as_u16(),
            serde_json::to_string(&second).unwrap(),
        ),
    );
    rig.app
        .dispatch(Action::Peers {
            action: wonton_protocol::PeerAction::ShowPeerCode,
        })
        .unwrap();
    wait_peers(&rig, |view| view.code.is_some()).await;

    rig.app
        .dispatch(Action::Peers {
            action: wonton_protocol::PeerAction::ResetPeerCode,
        })
        .unwrap();
    // 重置会先亮出新码，再重新取列表（先拨 Loading、取完才回 Ready）：等这两个条件一起成立。
    let view = wait_peers(&rig, |view| {
        view.state == PeersState::Ready
            && view
                .code
                .as_ref()
                .is_some_and(|code| code.code.expose().ends_with("two"))
    })
    .await;
    assert!(view.code.is_some());
    assert_eq!(
        rig.core.seen(&Method::GET, &peer_code_reset_path()).len(),
        0
    );
    assert_eq!(
        rig.core.seen(&Method::POST, &peer_code_reset_path()).len(),
        1
    );
}
