//! 同一个进程里，旧会话还没释放时在同一个存储上恢复会话：旧会话从备份取回密钥、把明文写回存储以后，
//! 新会话打开房间要看到明文，不能一直是“无法解密”。

use std::sync::Arc;

use axum::{
    Router,
    body::{Body, to_bytes},
    extract::{Request, State},
    response::Response,
};
use matrix_sdk::{reqwest, ruma::OwnedRoomId};
use tokio::{net::TcpListener, sync::watch, task::JoinHandle};
use wonton_matrix::{MatrixSession, MessageBody, Recovery, Verification};

use crate::{
    chat::is_text,
    support::{TempStore, TestServer, eventually, unique},
};

const HISTORY: &str = "恢复之前的历史";

#[tokio::test]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn a_restored_session_sees_history_the_old_session_decrypted() {
    let server = TestServer::start().await;
    let owner = unique("owner");
    let (room_id, key) = history_in_backup(&server, &owner).await;

    // 旧会话：输入恢复密钥，停同步，但一直不释放。它和新会话从备份取房间密钥的请求都先扣在各自的中继上。
    let store = TempStore::create();
    let old_relay = BackupGate::start(&server.url).await;
    let old = server
        .login_via(&old_relay.url, &owner, "第二台", store.config())
        .await;
    old.start_sync().await.expect("旧会话同步");
    wait_for(&old, "旧会话待恢复", |recovery, _| {
        recovery == Recovery::Incomplete
    })
    .await;
    old.recover(&key).await.expect("输入恢复密钥");
    let saved = old.saved().expect("登录状态");
    old.stop_sync().await;

    let new_relay = BackupGate::start(&server.url).await;
    let mut config = server.config("第二台", store.config());
    config.homeserver_url.clone_from(&new_relay.url);
    let new = MatrixSession::restore(&config, saved)
        .await
        .expect("恢复会话");
    new.start_sync().await.expect("新会话同步");
    wait_for(
        &new,
        "新会话已验证、备份已启用",
        |recovery, verification| {
            recovery == Recovery::Enabled && verification == Verification::Verified
        },
    )
    .await;
    let room = new.open_room(&room_id).await.expect("新会话打开私聊");
    room.paginate_backwards(20).await.expect("向前翻页");
    eventually(
        "新会话先是解不开",
        room.message_updates().await,
        |messages| {
            messages
                .iter()
                .any(|message| message.body == MessageBody::Undecryptable)
                .then_some(())
        },
    )
    .await;

    // 只放行旧会话：它取回密钥、解开消息并写回存储。存储先写，时间线后更新，看到明文时存储里已经是明文。
    old_relay.release();
    let old_room = old.open_room(&room_id).await.expect("旧会话打开私聊");
    wait_for_history(&old_room, "旧会话解开并写回").await;

    // 新会话自己的取密钥请求还扣着，只能从存储里看到旧会话写回的明文。
    drop(room);
    let room = new.open_room(&room_id).await.expect("新会话再打开私聊");
    wait_for_history(&room, "新会话看到旧会话写回的明文").await;
}

/// 第一台设备和 Bot 建加密私聊，Bot 发一条消息；第一台开恢复、备份上传完，然后都下线。返回私聊和恢复密钥。
async fn history_in_backup(server: &TestServer, owner: &str) -> (OwnedRoomId, String) {
    let first = server.login(owner, "第一台").await;
    first.start_sync().await.expect("第一台同步");
    wait_for(&first, "第一台已验证", |_, verification| {
        verification == Verification::Verified
    })
    .await;
    let bot = server.login(&unique("bot"), "Bot").await;
    bot.start_sync().await.expect("Bot 同步");
    let room_id = crate::chat::direct_chat(&first, &bot, true).await;
    let bot_room = bot.open_room(&room_id).await.expect("Bot 打开私聊");
    bot_room.send_text(HISTORY).await.expect("Bot 发消息");
    let room = first.open_room(&room_id).await.expect("第一台打开私聊");
    wait_for_history(&room, "第一台解开历史").await;
    // 恢复密钥在备份上传完以后才返回。
    let key = first.enable_recovery().await.expect("开启恢复");
    first.stop_sync().await;
    bot.stop_sync().await;
    (room_id, key)
}

async fn wait_for(
    session: &MatrixSession,
    what: &str,
    mut check: impl FnMut(Recovery, Verification) -> bool,
) {
    eventually(what, session.encryption_states(), |state| {
        check(state.recovery, state.verification).then_some(())
    })
    .await;
}

async fn wait_for_history(room: &wonton_matrix::ChatRoom, what: &str) {
    eventually(what, room.message_updates().await, |messages| {
        messages
            .iter()
            .any(|message| is_text(message, HISTORY))
            .then_some(())
    })
    .await;
}

/// 转发到 Tuwunel，按房间密钥从备份取（`GET /room_keys/keys/...`）的应答扣住，直到 [`Self::release`]。
struct BackupGate {
    url: String,
    release: watch::Sender<bool>,
    task: JoinHandle<()>,
}

struct GateState {
    upstream: String,
    http: reqwest::Client,
    release: watch::Receiver<bool>,
}

impl BackupGate {
    async fn start(upstream: &str) -> Self {
        let (release, released) = watch::channel(false);
        let state = Arc::new(GateState {
            upstream: upstream.to_owned(),
            http: reqwest::Client::new(),
            release: released,
        });
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let app = Router::new().fallback(forward).with_state(state);
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        Self { url, release, task }
    }

    fn release(&self) {
        self.release.send_replace(true);
    }
}

impl Drop for BackupGate {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn forward(State(state): State<Arc<GateState>>, request: Request) -> Response {
    let (parts, body) = request.into_parts();
    let held = parts.method == "GET" && parts.uri.path().contains("/room_keys/keys/");
    let body = to_bytes(body, usize::MAX).await.unwrap();
    let upstream = state
        .http
        .request(parts.method, format!("{}{}", state.upstream, parts.uri))
        .headers(parts.headers)
        .body(body)
        .send()
        .await
        .unwrap();
    let status = upstream.status();
    let headers = upstream.headers().clone();
    let body = upstream.bytes().await.unwrap();
    if held {
        let mut release = state.release.clone();
        // 测试结束时中继被丢掉，扣着的请求跟着结束。
        let _ = release.wait_for(|released| *released).await;
    }
    let mut response = Response::new(Body::from(body));
    *response.status_mut() = status;
    *response.headers_mut() = headers;
    response.headers_mut().remove("transfer-encoding");
    response
}
