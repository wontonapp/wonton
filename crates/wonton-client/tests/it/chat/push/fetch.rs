//! 推送取不到消息时照实说取不到，不显示成正常消息。

use super::*;

#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn a_push_that_cannot_be_fetched_says_why() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let home = Home::start(&[phone.device_id()]).await;
    phone.connect(&home);
    phone.signed_in().await;

    // 推送说的消息服务器上没有（比如被伪造的推送）：不显示成正常的消息，照实说取不到。
    let forged = json!({
        "server": home.server_id(),
        "room_id": "!nowhere:wonton.localhost",
        "event_id": "$forged",
        "sender_display_name": "Hermes",
    });
    let notification = phone
        .resolve_push(forged.to_string().into_bytes())
        .await
        .expect("格式是对的")
        .expect("要告诉主人有推送");
    assert_eq!(notification.sender.as_deref(), Some("Hermes"));
    assert!(
        matches!(
            notification.content,
            PushContent::Unavailable {
                reason: ChatFailure::NotReady | ChatFailure::Upstream,
                ..
            }
        ),
        "{:?}",
        notification.content
    );
}
