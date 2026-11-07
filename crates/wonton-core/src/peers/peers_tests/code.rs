//! 码的行为测试：长期有效、重置作废，以及码里有什么。

use super::*;

/// 重置码：换一个新令牌，旧码作废，用旧码发来的待处理请求一并清掉。
#[tokio::test]
async fn resetting_the_code_revokes_the_old_one_and_clears_requests() {
    let a = TestCore::start("Alex", "a.example.internal").await;
    let b = TestCore::start("Jiale", "b.example.internal").await;

    let old = a.peers.issue_code();
    b.scan_code(&old, &a).await.unwrap();
    a.request_now("b.example.internal");

    let fresh = a.peers.reset_code();
    assert_ne!(fresh.code.expose(), old.code.expose());
    assert!(a.peers.view(&langs()).requests.is_empty(), "旧码的请求清掉");

    let error = b.scan_code(&old, &a).await.unwrap_err();
    assert_eq!(error, PeerError::CodeRevoked);
    b.scan_code(&fresh, &a).await.unwrap();
    a.request_now("b.example.internal");
}

/// 码长期有效：不重置就一直能取到同一张，重启后也还是它。
#[tokio::test]
async fn the_code_stays_the_same_until_reset() {
    let mut a = TestCore::start("Alex", "a.example.internal").await;
    let first = a.peers.issue_code();
    let second = a.peers.issue_code();
    assert_eq!(first.code.expose(), second.code.expose());

    let (reopened, _) = a.reopen().await;
    let reopened_code = reopened.issue_code();
    let token = |code: &str| {
        wonton_tunnel::peer_pair::PeerPairingCode::parse(code)
            .unwrap()
            .token
            .expose()
            .to_owned()
    };
    assert_eq!(
        token(reopened_code.code.expose()),
        token(first.code.expose()),
        "重启后还是同一个令牌"
    );
    assert_ne!(
        token(reopened.reset_code().code.expose()),
        token(first.code.expose()),
        "重置以后就是新的"
    );
}

/// 码里没有直连地址：只有 endpoint ID、中继、服务器名和令牌。
#[tokio::test]
async fn the_code_has_no_direct_address() {
    let a = TestCore::start("Alex", "a.example.internal").await;
    let code = a.peers.issue_code();
    assert!(
        !code.code.expose().contains("addr="),
        "{}",
        code.code.expose()
    );
    assert!(code.code.expose().starts_with("wonton://peer?v=1&"));
}
