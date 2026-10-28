//! `echo-bot run`：用核心交给插件的令牌恢复会话，把主人的文字消息原样回过去。

use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use anyhow::{Context, bail};
use matrix_sdk::authentication::matrix::MatrixSession;
use matrix_sdk::config::SyncSettings;
use matrix_sdk::reqwest;
use matrix_sdk::room::Room;
use matrix_sdk::ruma::events::room::member::StrippedRoomMemberEvent;
use matrix_sdk::ruma::events::room::message::{
    MessageType, OriginalSyncRoomMessageEvent, RoomMessageEventContent,
};
use matrix_sdk::ruma::{OwnedDeviceId, OwnedUserId};
use matrix_sdk::store::RoomLoadSettings;
use matrix_sdk::{Client, LoopCtrl, RoomState, SessionMeta, SessionTokens};

use crate::PREFIX;

/// 最近一次同步成功的时间记在这里，健康检查看它。
const ALIVE: &str = "/tmp/echo-bot.alive";

/// 多久没同步成功就算不健康：长轮询 30 秒一次，留出余量。
const STALE: Duration = Duration::from_secs(90);

fn env(name: &str) -> anyhow::Result<String> {
    std::env::var(name).with_context(|| format!("缺少环境变量 {name}：要由核心安装插件时注入"))
}

pub async fn run() -> anyhow::Result<()> {
    let homeserver = env("WONTON_MATRIX_HOMESERVER")?;
    let token = env("WONTON_MATRIX_ACCESS_TOKEN")?;
    let owner: OwnedUserId = env("WONTON_MATRIX_OWNER")?
        .parse()
        .context("WONTON_MATRIX_OWNER 不是合法的用户 ID")?;
    let expected: OwnedUserId = env("WONTON_MATRIX_USER_ID")?
        .parse()
        .context("WONTON_MATRIX_USER_ID 不是合法的用户 ID")?;
    let (user_id, device_id) = whoami(&homeserver, &token).await?;
    if user_id != expected {
        bail!("令牌属于 {user_id}，不是 WONTON_MATRIX_USER_ID 里的 {expected}");
    }
    let store = PathBuf::from(std::env::var("ECHO_STORE").unwrap_or_else(|_| "/data".into()))
        .join(device_id.as_str());
    let client = Client::builder()
        .homeserver_url(&homeserver)
        .sqlite_store(&store, None)
        .build()
        .await
        .context("建 Matrix 客户端")?;
    let session = MatrixSession {
        meta: SessionMeta { user_id, device_id },
        tokens: SessionTokens {
            access_token: token,
            refresh_token: None,
        },
    };
    client
        .matrix_auth()
        .restore_session(session, RoomLoadSettings::default())
        .await
        .context("恢复会话")?;

    // 先同步一次再挂上处理函数：启动时同步回来的旧消息不回声。
    let first = client
        .sync_once(SyncSettings::default())
        .await
        .context("首次同步")?;
    touch_alive();
    tracing::info!(user = %client.user_id().map(ToString::to_string).unwrap_or_default(), "已连上 Matrix");
    let echo_owner = owner.clone();
    client.add_event_handler(move |event: OriginalSyncRoomMessageEvent, room: Room| {
        let owner = echo_owner.clone();
        async move { echo(event, room, &owner).await }
    });
    client.add_event_handler(
        move |event: StrippedRoomMemberEvent, room: Room, client: Client| {
            let owner = owner.clone();
            async move { accept_invite(event, room, client, &owner).await }
        },
    );
    let settings = SyncSettings::default().token(first.next_batch);
    client
        .sync_with_callback(settings, |_| async {
            touch_alive();
            LoopCtrl::Continue
        })
        .await
        .context("同步中断")
}

/// 主人的文字消息原样回过去；别人的、自己的、不是文字的都不理。
async fn echo(event: OriginalSyncRoomMessageEvent, room: Room, owner: &OwnedUserId) {
    if room.state() != RoomState::Joined || event.sender != *owner {
        return;
    }
    let MessageType::Text(text) = event.content.msgtype else {
        return;
    };
    let reply = RoomMessageEventContent::text_plain(format!("{PREFIX}{}", text.body));
    if let Err(e) = room.send(reply).await {
        tracing::warn!(room = %room.room_id(), error = %e, "回声没发出去");
    }
}

/// 主人邀请 Bot 进别的房间时接受；别人的邀请不理。
async fn accept_invite(
    event: StrippedRoomMemberEvent,
    room: Room,
    client: Client,
    owner: &OwnedUserId,
) {
    let invited_me = client.user_id().is_some_and(|me| event.state_key == me);
    if !invited_me || event.sender != *owner || room.state() != RoomState::Invited {
        return;
    }
    if let Err(e) = room.join().await {
        tracing::warn!(room = %room.room_id(), error = %e, "接受邀请失败");
    }
}

/// 令牌属于哪个用户、哪台设备：恢复会话要设备 ID，核心只注入了令牌。
async fn whoami(homeserver: &str, token: &str) -> anyhow::Result<(OwnedUserId, OwnedDeviceId)> {
    let url = format!(
        "{}/_matrix/client/v3/account/whoami",
        homeserver.trim_end_matches('/')
    );
    let response = reqwest::Client::new()
        .get(&url)
        .bearer_auth(token)
        .send()
        .await
        .with_context(|| format!("连不上 {homeserver}"))?;
    let status = response.status();
    let bytes = response.bytes().await.context("读 whoami 的响应")?;
    let body: serde_json::Value =
        serde_json::from_slice(&bytes).context("whoami 的响应不是 JSON")?;
    if !status.is_success() {
        bail!("令牌不能用（HTTP {status}）：{body}");
    }
    let field = |name: &str| {
        body[name]
            .as_str()
            .with_context(|| format!("whoami 的响应缺少 {name}：{body}"))
    };
    let user = field("user_id")?.parse().context("user_id 不合法")?;
    let device = field("device_id")?.into();
    Ok((user, device))
}

fn touch_alive() {
    if let Err(e) = std::fs::write(ALIVE, b"") {
        tracing::warn!(error = %e, "写不了 {ALIVE}");
    }
}

pub fn health() -> anyhow::Result<()> {
    let modified = std::fs::metadata(ALIVE)
        .and_then(|m| m.modified())
        .context("还没同步成功过")?;
    let age = SystemTime::now()
        .duration_since(modified)
        .unwrap_or_default();
    if age > STALE {
        bail!("{} 秒没同步成功", age.as_secs());
    }
    Ok(())
}
