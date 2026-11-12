use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use serde_json::json;
use tokio::io::AsyncWriteExt;
use wonton_protocol::MAIL_FIELD;

use super::test_support::{Mailpit, PASSWORD, Signal, home, received};

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
    mailpit.events.send(Signal::New("live".into())).unwrap();
    until(|| mailpit.state.lock().unwrap().deleted == ["startup", "live"]).await;
    mailpit.state.lock().unwrap().fail_list = false;

    mailpit.events.send(Signal::Close).unwrap();
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

#[tokio::test]
async fn a_socket_drop_without_a_close_frame_reconnects_and_backfills_each_mail_once() {
    let home = home().await;
    let mailpit = Mailpit::start().await;
    let to = json!([{"Address":"owner@wonton.internal"}]);
    mailpit.add("first", to.clone(), received("172.30.1.5"));
    let worker = super::start(
        &mailpit.authority,
        PASSWORD,
        crate::http::HttpClient::new(),
        home.gateway.clone(),
        home.dir.path(),
    )
    .unwrap();
    until(|| mailpit.state.lock().unwrap().deleted == ["first"]).await;

    // Mailpit 收不到 pong 或重启时直接关 TCP、不发 close frame；断开期间来的信没有事件，
    // 重连后的补漏必须把它投出去，且已投过的不再投第二遍。
    mailpit.events.send(Signal::Drop).unwrap();
    mailpit.add("missed", to, received("172.30.1.5"));
    until(|| mailpit.state.lock().unwrap().deleted == ["first", "missed"]).await;
    assert!(mailpit.state.lock().unwrap().connections >= 2);
    drop(worker);
    let matrix = home.matrix.state();
    assert_eq!(matrix.sent.len(), 2);
    for (room, content) in &matrix.sent {
        assert_eq!(room, &matrix.sent[0].0);
        assert_eq!(content[MAIL_FIELD]["plugin_id"], "app.test.mail");
    }
}

#[tokio::test]
async fn a_worker_started_while_mailpit_is_not_ready_connects_once_it_becomes_ready() {
    let home = home().await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let authority = listener.local_addr().unwrap().to_string();
    // 还没就绪的样子：接下 TCP 就立刻断开，核心侧是传输不可用，不是 Mailpit 的错。
    let (ready_tx, mut ready_rx) = tokio::sync::oneshot::channel::<()>();
    let refused = Arc::new(AtomicUsize::new(0));
    let counter = refused.clone();
    let not_ready = tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = &mut ready_rx => return listener,
                accepted = listener.accept() => {
                    if let Ok((mut socket, _)) = accepted {
                        counter.fetch_add(1, Ordering::SeqCst);
                        let _ = socket.shutdown().await;
                    }
                }
            }
        }
    });
    let worker = super::start(
        &authority,
        PASSWORD,
        crate::http::HttpClient::new(),
        home.gateway.clone(),
        home.dir.path(),
    )
    .unwrap();

    // 吃过至少一次连接失败，才真的覆盖了「没就绪时重试」。
    until(|| refused.load(Ordering::SeqCst) >= 1).await;
    ready_tx.send(()).unwrap();
    let mailpit = Mailpit::serve_on(not_ready.await.unwrap()).await;
    mailpit.add(
        "startup",
        json!([{"Address":"owner@wonton.internal"}]),
        received("172.30.1.5"),
    );

    until(|| mailpit.state.lock().unwrap().deleted == ["startup"]).await;
    drop(worker);
    assert_eq!(home.matrix.state().sent.len(), 1);
}
