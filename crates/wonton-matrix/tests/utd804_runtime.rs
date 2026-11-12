#![recursion_limit = "256"]

use wonton_matrix::testkit::{TestServer, eventually, unique};
use wonton_matrix::{MatrixSession, MessageBody, Recovery, StoreConfig, Verification};

mod utd804_support;
use utd804_support::Relay;

#[test]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
fn fresh_runtime_first_open_after_restart() {
    tracing_subscriber::fmt().with_ansi(false).with_writer(std::io::stderr)
        .with_env_filter(tracing_subscriber::EnvFilter::new("warn,matrix_sdk::encryption=debug,matrix_sdk::event_cache=debug,matrix_sdk::event_cache::redecryptor=trace,matrix_sdk_crypto::machine=debug,matrix_sdk_crypto::store=debug,matrix_sdk_ui::timeline=debug"))
        .try_init().unwrap();
    let server_runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2).enable_all().build().unwrap();
    let server = server_runtime.block_on(TestServer::start());
    let relay = server_runtime.block_on(Relay::start(&server.url));
    let second_dir = tempfile::tempdir().unwrap();
    let store = StoreConfig::Sqlite {
        path: second_dir.path().to_path_buf(), passphrase: None,
    };
    let app_runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all().build().unwrap();
    let (saved, room_id) = app_runtime.block_on(async {
        let owner = unique("owner");
        let first = server.login(&owner, "first").await;
        first.start_sync().await.unwrap();
        eventually("first identity", first.encryption_states(), |state| {
            (state.verification == Verification::Verified).then_some(())
        }).await;
        let bot = server.login(&unique("bot"), "bot").await;
        bot.start_sync().await.unwrap();
        let room_id = first.create_direct_chat(bot.user_id(), true).await.unwrap();
        eventually("bot invite", bot.rooms().await.unwrap(), |rooms| {
            rooms.iter().any(|room| room.room_id == room_id).then_some(())
        }).await;
        bot.join_room(&room_id).await.unwrap();
        let room = first.open_room(&room_id).await.unwrap();
        let bot_room = bot.open_room(&room_id).await.unwrap();
        bot_room.send_text("history before recovery").await.unwrap();
        eventually("first device reads encrypted history", room.message_updates().await, |messages| {
            messages.iter().any(|message| message.body == MessageBody::Text("history before recovery".to_owned())).then_some(())
        }).await;
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
        relay.phase(1, "second login before recovery");
        let second = server.login_via(&relay.url, &owner, "second", store.clone()).await;
        second.start_sync().await.unwrap();
        eventually("second needs recovery", second.encryption_states(), |state| {
            (state.recovery == Recovery::Incomplete).then_some(())
        }).await;
        relay.phase(2, "recover once");
        second.recover(&key).await.unwrap();
        println!("SMOKE recovery-key import completed: {:?}", second.encryption_state());
        let saved = second.saved().unwrap();
        second.stop_sync().await;
        drop(second);
        (saved, room_id)
    });
    drop(app_runtime);
    eprintln!("TRACE804 OLD_APP_RUNTIME_DROPPED; no old client task can run");
    let restarted_relay = server_runtime.block_on(Relay::start(&server.url));
    restarted_relay.phase(40, "restored client in fresh runtime");
    let restored_runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all().build().unwrap();
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        restored_runtime.block_on(async {
            let mut config = server.config("second", store);
            config.homeserver_url = restarted_relay.url.clone();
            let second = MatrixSession::restore(&config, saved).await.unwrap();
            second.start_sync().await.unwrap();
            eventually("recovery survives restart", second.encryption_states(), |state| {
                (state.verification == Verification::Verified && state.recovery == Recovery::Enabled).then_some(())
            }).await;
            println!("SMOKE first history open after restart: {:?}", second.encryption_state());
            let restored_room = second.open_room(&room_id).await.unwrap();
            restored_room.paginate_backwards(20).await.unwrap();
            eventually("history decrypts after recovery and restart", restored_room.message_updates().await, |messages| {
                messages.iter().any(|message| message.body == MessageBody::Text("history before recovery".to_owned())).then_some(())
            }).await;
            let timeline = restored_room.timeline().await;
            let sdk_room = timeline.room();
            let (cache, _handles) = sdk_room.client().event_cache().room(sdk_room.room_id()).await.unwrap();
            for event in cache.events().await.unwrap() {
                eprintln!("TRACE804 FRESH_CACHE event_id={:?} kind={:?}", event.event_id(), event.kind);
            }
            for item in timeline.items().await {
                if let Some(event) = item.as_event() {
                    eprintln!("TRACE804 FRESH_UI event_id={:?} content={:?}", event.event_id(), event.content());
                }
            }
            second.stop_sync().await;
            println!("SMOKE PASS fresh runtime first history open after recovery and restart");
        });
    }));
    drop(restored_runtime);
    drop(restarted_relay);
    drop(relay);
    server_runtime.block_on(async move { drop(server); });
    drop(server_runtime);
    if let Err(panic) = outcome { std::panic::resume_unwind(panic); }
}
