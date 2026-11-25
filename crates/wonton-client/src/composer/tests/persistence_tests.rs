use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn the_tray_and_pending_messages_survive_a_restart() {
    let s = setup();
    s.link.add_image(ROOM, photo("a.jpg")).unwrap();
    s.link.send(ROOM, "重启后发", None, &[]).unwrap();
    s.link.add_image(ROOM, photo("b.jpg")).unwrap();
    until("在传", || s.fake.upload_calls() == 2).await;

    // App 被杀：换一个核心读同一个目录。
    let fake = Fake::new();
    let link = ComposerLink::new(
        tokio::runtime::Handle::current(),
        fake.clone(),
        s._dir.path(),
    );
    let restored = link.subscribe(ROOM).borrow().clone();
    assert_eq!(restored.outgoing.len(), 1);
    assert_eq!(restored.outgoing[0].body, "重启后发");
    assert_eq!(restored.attachments.len(), 1, "附件区也还在");
    until("照片从头重传", || fake.upload_calls() == 2).await;
    fake.finish("a.jpg", Ok(uploaded("a.jpg", true)));
    until("接着发", || fake.groups() == 1).await;
    assert_eq!(
        fake.groups.lock().unwrap()[0].parts,
        [image(uploaded("a.jpg", true), Some("重启后发"))]
    );
}

/// 旧的 `composer.json` 里 `failure` 是一句原文：解析不能整份失败，原文要留着。
#[test]
fn an_old_failure_string_still_loads() {
    let dir = tempfile::tempdir().unwrap();
    let state = serde_json::json!({
        "rooms": {
            "!room:example.invalid": {
                "tray": [],
                "outgoing": [{
                    "id": "abc",
                    "group": "abc",
                    "room": "!room:example.invalid",
                    "body": "hello",
                    "reply_to": null,
                    "prepared_reply": null,
                    "items": [],
                    "timestamp_ms": 1,
                    "handed": 0,
                    "failure": "transport unavailable"
                }]
            }
        }
    });
    std::fs::write(dir.path().join("composer.json"), state.to_string()).unwrap();
    let loaded = super::store::Disk::new(dir.path().to_path_buf()).load();
    let room = loaded
        .rooms
        .get("!room:example.invalid")
        .expect("the room survives");
    let failure = room.outgoing[0]
        .failure
        .as_ref()
        .expect("the old failure text survives");
    assert!(matches!(
        failure.failure,
        wonton_protocol::SendFailure::Other
    ));
    assert_eq!(failure.details, "transport unavailable");
}
