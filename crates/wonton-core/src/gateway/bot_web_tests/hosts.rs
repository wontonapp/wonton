//! 每个 Bot 主机只到自己上游，元数据给出网页地址。

use super::*;

#[tokio::test]
async fn two_bot_hosts_reach_only_their_own_upstream_and_metadata() {
    let h = Harness::start(None).await;
    let other_hits = setup(&h).await;
    let first = h.app_session_on(&host("default")).await;
    let second = h.app_session_on(&host("mia")).await;
    let a = h
        .get(&host("default"), "/first", &[("cookie", &first)])
        .await;
    let b = h.get(&host("mia"), "/second", &[("cookie", &second)]).await;
    assert_eq!(a.status, StatusCode::OK);
    assert_eq!(b.status, StatusCode::OK);
    assert_eq!(h.hits.load(Ordering::SeqCst), 1);
    assert_eq!(other_hits.load(Ordering::SeqCst), 1);
    assert_eq!(
        a.headers["document-isolation-policy"],
        "isolate-and-credentialless"
    );
    for name in ["default", "mia"] {
        let result = h
            .get(
                CORE,
                &bot_web_path(&bot(name)),
                &[("authorization", &format!("Bearer {TOKEN}"))],
            )
            .await;
        assert_eq!(result.status, StatusCode::OK, "{}", result.text());
        assert_eq!(
            result.json()["page"]["bot"],
            serde_json::to_value(bot(name)).unwrap()
        );
        assert_eq!(
            result.json()["page"]["url"],
            format!("{}/panel?mode=bot", h.origin(&host(name)))
        );
    }
}
