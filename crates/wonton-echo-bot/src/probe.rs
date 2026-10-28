//! `echo-bot probe`：冒烟测试里扮演主人的一台设备。
//!
//! ```text
//! echo-bot probe --homeserver <地址> [--proxy <代理>] --login-token <JWT> --room <房间> --bot <Bot 的用户 ID> [--text <文字>]
//! ```
//!
//! 用核心签的登录令牌登录一台临时设备（密钥只在内存里），确认主房间是加密的，发一条消息，等 Bot 的回声：
//! 回声必须是加密消息，内容对得上。结束时退出登录，设备一起删掉。

use std::time::Duration;

use anyhow::{Context, bail};
use matrix_sdk::config::SyncSettings;
use matrix_sdk::deserialized_responses::EncryptionInfo;
use matrix_sdk::room::Room;
use matrix_sdk::ruma::events::room::message::{
    MessageType, OriginalSyncRoomMessageEvent, RoomMessageEventContent,
};
use matrix_sdk::ruma::serde::JsonObject;
use matrix_sdk::ruma::{OwnedRoomId, OwnedUserId};
use matrix_sdk::{Client, RoomState};
use tokio::sync::mpsc;

use crate::PREFIX;

/// 等回声的上限：Bot 要先拿到这台新设备的密钥才解得开消息。
const WAIT: Duration = Duration::from_secs(60);

struct Options {
    homeserver: String,
    proxy: Option<String>,
    token: String,
    room: OwnedRoomId,
    bot: OwnedUserId,
    text: String,
}

impl Options {
    fn parse(args: &[String]) -> anyhow::Result<Self> {
        let mut values = std::collections::BTreeMap::new();
        let mut rest = args.iter();
        while let Some(flag) = rest.next() {
            let name = flag
                .strip_prefix("--")
                .with_context(|| format!("不认识的参数 {flag}"))?;
            let value = rest.next().with_context(|| format!("{flag} 后面要有值"))?;
            values.insert(name.to_owned(), value.clone());
        }
        let mut take = |name: &str| {
            values
                .remove(name)
                .with_context(|| format!("缺少 --{name}"))
        };
        let options = Self {
            homeserver: take("homeserver")?,
            token: take("login-token")?,
            room: take("room")?.parse().context("--room 不是合法的房间 ID")?,
            bot: take("bot")?.parse().context("--bot 不是合法的用户 ID")?,
            proxy: take("proxy").ok(),
            text: take("text").unwrap_or_else(|_| "你好，Wonton".to_owned()),
        };
        if let Some(extra) = values.keys().next() {
            bail!("不认识的参数 --{extra}");
        }
        Ok(options)
    }
}

pub async fn run(args: &[String]) -> anyhow::Result<()> {
    let options = Options::parse(args)?;
    let mut builder = Client::builder().homeserver_url(&options.homeserver);
    if let Some(proxy) = &options.proxy {
        builder = builder.proxy(proxy);
    }
    let client = builder.build().await.context("建 Matrix 客户端")?;
    let data = JsonObject::from_iter([("token".to_owned(), options.token.clone().into())]);
    client
        .matrix_auth()
        .login_custom("org.matrix.login.jwt", data)
        .context("登录请求不对")?
        .initial_device_display_name("Wonton 冒烟")
        .await
        .context("用登录令牌登录")?;
    let result = exchange(&client, &options).await;
    if let Err(e) = client.matrix_auth().logout().await {
        tracing::warn!(error = %e, "冒烟设备退出登录失败");
    }
    result
}

/// 发一条加密消息，等 Bot 的加密回声。
async fn exchange(client: &Client, options: &Options) -> anyhow::Result<()> {
    let first = client
        .sync_once(SyncSettings::default())
        .await
        .context("首次同步")?;
    let room = client
        .get_room(&options.room)
        .filter(|room| room.state() == RoomState::Joined)
        .with_context(|| format!("主人不在房间 {} 里", options.room))?;
    if !room.latest_encryption_state().await?.is_encrypted() {
        bail!("房间 {} 没有加密", options.room);
    }
    let (sender, mut replies) = mpsc::unbounded_channel();
    let (room_id, bot) = (options.room.clone(), options.bot.clone());
    client.add_event_handler(
        move |event: OriginalSyncRoomMessageEvent,
              room: Room,
              encryption: Option<EncryptionInfo>| {
            let sender = sender.clone();
            let from_bot = room.room_id() == room_id && event.sender == bot;
            async move {
                if let (true, MessageType::Text(text)) = (from_bot, event.content.msgtype) {
                    let _ = sender.send((text.body, encryption.is_some()));
                }
            }
        },
    );
    // 同步和等回声在同一个任务里并行：matrix-sdk 的同步 future 太深，放进 tokio::spawn 编译器证不出它是 Send。
    let settings = SyncSettings::default().token(first.next_batch);
    let sync = client.sync(settings);
    tokio::pin!(sync);
    room.send(RoomMessageEventContent::text_plain(&options.text))
        .await
        .context("发消息")?;
    let expected = format!("{PREFIX}{}", options.text);
    let wait = tokio::time::timeout(WAIT, async {
        while let Some((body, encrypted)) = replies.recv().await {
            if body == expected {
                return Some(encrypted);
            }
        }
        None
    });
    let answer = tokio::select! {
        stopped = &mut sync => bail!("同步中断：{stopped:?}"),
        answer = wait => answer,
    };
    match answer {
        Ok(Some(true)) => {
            println!("收到加密的回声：{expected}");
            Ok(())
        }
        Ok(Some(false)) => bail!("回声没有加密"),
        Ok(None) | Err(_) => bail!(
            "{} 秒内没收到 {} 的回声（Bot 没在运行，或者解不开这台设备的加密消息）",
            WAIT.as_secs(),
            options.bot
        ),
    }
}
