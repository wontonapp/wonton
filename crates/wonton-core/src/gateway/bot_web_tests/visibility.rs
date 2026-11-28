//! 可见性按角色决定，family 标记不再管事。

use super::*;

#[tokio::test]
async fn family_switch_not_bot_keeper_controls_both_web_and_metadata() {
    let h = Harness::start(None).await;
    setup(&h).await;
    let device = member(&h);
    for name in ["default", "mia"] {
        for (target, path) in [
            (CORE.to_owned(), bot_web_path(&bot(name))),
            (host(name), "/".into()),
        ] {
            let reply = send_as(&h.app, Some(device), Method::GET, &target, &path, &[], "").await;
            assert_failure(&reply, StatusCode::FORBIDDEN, "not_shared");
        }
    }
    open_whoami(&h);
    for name in ["default", "mia"] {
        let reply = send_as(&h.app, Some(device), Method::GET, &host(name), "/", &[], "").await;
        assert_eq!(reply.status, StatusCode::OK, "{}", reply.text());
    }
    h.tunnel.devices.remove(&device).unwrap();
    let reply = send_as(
        &h.app,
        Some(device),
        Method::GET,
        CORE,
        &bot_web_path(&bot("mia")),
        &[],
        "",
    )
    .await;
    assert_failure(&reply, StatusCode::UNAUTHORIZED, "unauthenticated");
}
