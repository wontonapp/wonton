#![recursion_limit = "256"]

use wonton_matrix::testkit::{TestServer, eventually, unique};
use wonton_matrix::{MatrixSession, MessageBody, Recovery, StoreConfig, Verification};

mod utd804_support;
use utd804_support::Relay;

#[tokio::main(flavor = "current_thread")]
async fn run(open_before_restart: bool) {
    tracing_subscriber::fmt().with_ansi(false).with_writer(std::io::stderr)
        .with_env_filter(tracing_subscriber::EnvFilter::new("warn,matrix_sdk::encryption=debug,matrix_sdk::event_cache=debug,matrix_sdk::event_cache::redecryptor=trace,matrix_sdk::sync=debug,matrix_sdk_crypto::machine=debug,matrix_sdk_crypto::store=debug,matrix_sdk_ui::timeline=trace"))
        .try_init().unwrap();
    eprintln!("TRACE804 MODE open_before_restart={open_before_restart}");
    let server = TestServer::start().await;
    let owner = unique("owner");
    let first = server.login(&owner, "first").await;
    first.start_sync().await.unwrap();
    eventually("first identity", first.encryption_states(), |state| {
        (state.verification == Verification::Verified).then_some(())
    })
    .await;
    let bot = server.login(&unique("bot"), "bot").await;
    bot.start_sync().await.unwrap();
    let room_id = first.create_direct_chat(bot.user_id(), true).await.unwrap();
    eventually("bot invite", bot.rooms().await.unwrap(), |rooms| {
        rooms
            .iter()
            .any(|room| room.room_id == room_id)
            .then_some(())
    })
    .await;
    bot.join_room(&room_id).await.unwrap();
    let room = first.open_room(&room_id).await.unwrap();
    let bot_room = bot.open_room(&room_id).await.unwrap();
    bot_room.send_text("history before recovery").await.unwrap();
    eventually(
        "first device reads encrypted history",
        room.message_updates().await,
        |messages| {
            messages
                .iter()
                .any(|message| {
                    message.body == MessageBody::Text("history before recovery".to_owned())
                })
                .then_some(())
        },
    )
    .await;
    println!("SMOKE first device created encrypted history");
    let key = first.enable_recovery().await.unwrap();
    let (access_token, _) = first.tokens().unwrap();
    let metadata = matrix_sdk::reqwest::Client::new()
        .get(format!("{}/_matrix/client/v3/room_keys/version", server.url))
        .bearer_auth(access_token).send().await.unwrap().error_for_status().unwrap()
        .text().await.unwrap();
    println!("SMOKE backup metadata: {metadata}");
    first.stop_sync().await;
    bot.stop_sync().await;
    drop(room);
    drop(bot_room);
    drop(first);
    drop(bot);
    let second_dir = tempfile::tempdir().unwrap();
    let store = StoreConfig::Sqlite {
        path: second_dir.path().to_path_buf(),
        passphrase: None,
    };
    let relay = Relay::start(&server.url).await;
    relay.phase(1, "second login before recovery");
    let second = server
        .login_via(&relay.url, &owner, "second", store.clone())
        .await;
    second.start_sync().await.unwrap();
    eventually(
        "second needs recovery",
        second.encryption_states(),
        |state| (state.recovery == Recovery::Incomplete).then_some(()),
    )
    .await;
    relay.phase(2, "recover once");
    second.recover(&key).await.unwrap();
    println!(
        "SMOKE recovery-key import completed: {:?}",
        second.encryption_state()
    );
    if open_before_restart {
        relay.phase(3, "first history open after recovery before restart");
        let history = second.open_room(&room_id).await.unwrap();
        history.paginate_backwards(20).await.unwrap();
        eventually("history decrypts before restart", history.message_updates().await, |messages| {
            messages.iter().any(|message| message.body == MessageBody::Text("history before recovery".to_owned())).then_some(())
        }).await;
        eprintln!("TRACE804 PRE_RESTART plaintext observed");
    }
    let saved = second.saved().unwrap();
    second.stop_sync().await;
    drop(second);
    let restarted_relay = Relay::start(&server.url).await;
    restarted_relay.phase(40, "restored client only");
    relay.phase(4, "restore saved second device");
    let mut restored_config = server.config("second", store);
    restored_config.homeserver_url = restarted_relay.url.clone();
    let second = MatrixSession::restore(&restored_config, saved)
        .await
        .unwrap();
    second.start_sync().await.unwrap();
    eventually(
        "recovery survives restart",
        second.encryption_states(),
        |state| {
            (state.verification == Verification::Verified && state.recovery == Recovery::Enabled)
                .then_some(())
        },
    )
    .await;
    println!("SMOKE first history open after restart: {:?}", second.encryption_state());
    relay.phase(5, "open and paginate history after restart");
    let restored_room = second.open_room(&room_id).await.unwrap();
    restored_room.paginate_backwards(20).await.unwrap();
    use futures_util::FutureExt;
    let outcome = std::panic::AssertUnwindSafe(eventually(
        "history decrypts after recovery and restart",
        restored_room.message_updates().await,
        |messages| {
            messages.iter().any(|message| message.body == MessageBody::Text("history before recovery".to_owned())).then_some(())
        },
    )).catch_unwind().await;
    let timeline = restored_room.timeline().await;
    let sdk_room = timeline.room();
    let (cache, _cache_handles) = sdk_room.client().event_cache().room(sdk_room.room_id()).await.unwrap();
    let cached = cache.events().await.unwrap();
    for event in &cached {
        eprintln!("TRACE804 FINAL_CACHE event_id={:?} kind={:?}", event.event_id(), event.kind);
    }
    {
        let client = sdk_room.client();
        let store_lock = client.event_cache_store().lock().await.unwrap();
        let persisted = store_lock.as_clean().expect("clean cache store");
        for event in &cached {
            if let Some(event_id) = event.event_id() {
                if let Some(stored) = persisted.find_event(sdk_room.room_id(), event_id).await.unwrap() {
                    eprintln!("TRACE804 FINAL_STORE event_id={event_id} kind={:?}", stored.kind);
                }
            }
            if matches!(event.kind, matrix_sdk::deserialized_responses::TimelineEventKind::UnableToDecrypt { .. }) {
                let decrypted = sdk_room.decrypt_event(event.raw().cast_ref_unchecked(), None).await;
                eprintln!("TRACE804 DIRECT_DECRYPT result={decrypted:?}");
            }
        }
    }
    for item in timeline.items().await {
        if let Some(event) = item.as_event() {
            eprintln!("TRACE804 FINAL_UI event_id={:?} content={:?}", event.event_id(), event.content());
        }
    }
    if let Err(panic) = outcome { std::panic::resume_unwind(panic); }
    relay.phase(6, "plaintext observed after restart");
    println!("SMOKE PASS recovery enabled after restart and encrypted history decrypted");
}

#[test]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
fn history_first_open_after_restart() {
    run(false);
}

#[test]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
fn history_first_open_after_recovery_before_restart() {
    run(true);
}
