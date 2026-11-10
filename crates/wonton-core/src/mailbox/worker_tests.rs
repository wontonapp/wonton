use std::time::Duration;

use serde_json::json;
use wonton_protocol::MAIL_FIELD;

use super::test_support::{Mailpit, PASSWORD, home, received};

async fn until(mut state: impl FnMut() -> bool) {
    tokio::time::timeout(Duration::from_secs(20), async {
        while !state() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn startup_live_events_and_reconnect_backfill_deliver_each_mail_once() {
    let home = home().await;
    let mailpit = Mailpit::start().await;
    let to = json!([{"Address":"owner@wonton.internal"}]);
    mailpit.add("startup", to.clone(), received("172.30.1.5"));
    let worker = super::start(
        &mailpit.authority,
        PASSWORD,
        crate::http::HttpClient::new(),
        home.gateway.clone(),
        home.dir.path(),
    )
    .unwrap();
    until(|| mailpit.state.lock().unwrap().deleted == ["startup"]).await;
    assert_eq!(mailpit.state.lock().unwrap().connections, 1);

    // A new event must deliver even when the list API cannot supply that message.
    mailpit.state.lock().unwrap().fail_list = true;
    mailpit.add("live", to.clone(), received("172.30.1.5"));
    mailpit.events.send(Some("live".into())).unwrap();
    until(|| mailpit.state.lock().unwrap().deleted == ["startup", "live"]).await;
    mailpit.state.lock().unwrap().fail_list = false;

    mailpit.events.send(None).unwrap();
    until(|| mailpit.state.lock().unwrap().closed == 1).await;
    mailpit.add("missed", to, received("172.30.1.5"));
    // No event is sent for this mail; the new subscription's API sweep must find it.
    until(|| mailpit.state.lock().unwrap().deleted == ["startup", "live", "missed"]).await;
    assert!(mailpit.state.lock().unwrap().connections >= 2);
    drop(worker);
    let matrix = home.matrix.state();
    assert_eq!(matrix.sent.len(), 3);
    for (room, content) in &matrix.sent {
        assert_eq!(room, &matrix.sent[0].0);
        assert_eq!(content["msgtype"], "m.text");
        assert_eq!(content[MAIL_FIELD]["plugin_id"], "app.test.mail");
    }
}
