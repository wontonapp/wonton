//! `echo-bot probe`：冒烟测试里扮演主人的一台设备。
//!
//! ```text
//! echo-bot probe --homeserver <地址> [--proxy <代理>] --login-token <JWT> --room <房间> --bot <Bot 的用户 ID>
//!                [--text <文字>]… [--reply echo|any] [--match <文字>] [--wait <秒>] [--settle <秒>]
//!                [--thread <话题根事件 ID|auto>] [--thread-delay <秒>] [--print-event-id <任意值>] [--mention <用户 ID>]
//!                [--redacted <秒>] [--token-only <任意值>]
//! ```
//!
//! 用核心签的登录令牌登录一台临时设备（密钥只在内存里），确认房间是加密的，按顺序发每条 `--text`，每发一条等
//! Bot 回一条：`--reply echo`（默认）要求回 `回声：<原文>`，`--reply any` 只要回了文字就行（如 Hermes 这种模型
//! 驱动的 Bot）。回复必须是加密消息。`--settle` 时收到回复后接着收，直到 Bot 这么多秒不再说话：Bot 先说一句再去
//! 调要等主人确认的工具时，这样才收得到它最后的回答。结束时退出登录，设备一起删掉。

use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use anyhow::{Context, bail};
use matrix_sdk::config::SyncSettings;
use matrix_sdk::deserialized_responses::EncryptionInfo;
use matrix_sdk::room::Room;
use matrix_sdk::ruma::api::client::keys::get_keys;
use matrix_sdk::ruma::api::client::room::get_room_event;
use matrix_sdk::ruma::events::Mentions;
use matrix_sdk::ruma::events::relation::Thread;
use matrix_sdk::ruma::events::room::message::Relation;
use matrix_sdk::ruma::events::room::message::{
    AddMentions, MessageType, OriginalSyncRoomMessageEvent, ReplyMetadata, ReplyWithinThread,
    RoomMessageEventContent,
};
use matrix_sdk::ruma::serde::JsonObject;
use matrix_sdk::ruma::{OwnedEventId, OwnedRoomId, OwnedUserId};
use matrix_sdk::{Client, RoomState};
use tokio::sync::mpsc;

use crate::PREFIX;

/// 每一轮等回复的默认上限：Bot 要先拿到这台新设备的密钥才解得开消息。
const WAIT: Duration = Duration::from_secs(60);

/// 认得的参数。
const FLAGS: [&str; 16] = [
    "homeserver",
    "proxy",
    "login-token",
    "room",
    "bot",
    "text",
    "reply",
    "match",
    "wait",
    "settle",
    "thread",
    "thread-delay",
    "print-event-id",
    "mention",
    "redacted",
    "token-only",
];

#[derive(Clone, Copy, PartialEq, Eq)]
enum Reply {
    /// 回 `回声：<原文>`。
    Echo,
    /// 回了文字就行。
    Any,
}

struct Options {
    homeserver: String,
    proxy: Option<String>,
    token: String,
    room: OwnedRoomId,
    bot: OwnedUserId,
    texts: Vec<String>,
    reply: Reply,
    /// 只认含这段文字的回复（`--reply any` 时用来跳过 m.notice 之类的插话）。
    match_text: Option<String>,
    wait: Duration,
    settle: Option<Duration>,
    /// 把消息都发进这个根消息开的话题，也只收这个话题里 Bot 的回复；`auto` 是第一条消息自己
    /// 当根、后面的进它开的话题（同一台设备，免去第二台设备现建密钥）。
    thread: Option<String>,
    /// `auto` 时第一条发出后等多久再发第一条话题消息。
    thread_delay: Duration,
    /// 每发一条消息后打印它的事件 ID，供调用方在服务器上核对（如撤回）。
    print_event_id: bool,
    /// 每条消息都带上这个提及；群里的命令要提及 Bot 才算数。
    mention: Option<OwnedUserId>,
    /// 发完消息等服务器把它撤回（content 清空），最多等这么多秒。
    redacted: Option<Duration>,
    /// 只登录并打印访问令牌，不进房间。
    token_only: bool,
}

impl Options {
    fn parse(args: &[String]) -> anyhow::Result<Self> {
        let mut values: Vec<(String, String)> = Vec::new();
        let mut rest = args.iter();
        while let Some(flag) = rest.next() {
            let name = flag
                .strip_prefix("--")
                .filter(|name| FLAGS.contains(name))
                .with_context(|| format!("不认识的参数 {flag}"))?;
            let value = rest.next().with_context(|| format!("{flag} 后面要有值"))?;
            values.push((name.to_owned(), value.clone()));
        }
        let all = |name: &str| -> Vec<String> {
            values
                .iter()
                .filter(|(n, _)| n == name)
                .map(|(_, v)| v.clone())
                .collect()
        };
        let one = |name: &str| all(name).pop();
        let required = |name: &str| one(name).with_context(|| format!("缺少 --{name}"));
        let mut texts = all("text");
        if texts.is_empty() {
            texts.push("你好，Wonton".to_owned());
        }
        let reply = match one("reply").as_deref() {
            None | Some("echo") => Reply::Echo,
            Some("any") => Reply::Any,
            Some(other) => bail!("--reply 只能是 echo 或 any，不是 {other}"),
        };
        let wait = match one("wait") {
            Some(secs) => Duration::from_secs(secs.parse().context("--wait 要是秒数")?),
            None => WAIT,
        };
        let settle = match one("settle") {
            Some(secs) => Some(Duration::from_secs(
                secs.parse().context("--settle 要是秒数")?,
            )),
            None => None,
        };
        Ok(Self {
            homeserver: required("homeserver")?,
            token: required("login-token")?,
            room: required("room")?
                .parse()
                .context("--room 不是合法的房间 ID")?,
            bot: required("bot")?
                .parse()
                .context("--bot 不是合法的用户 ID")?,
            proxy: one("proxy"),
            texts,
            reply,
            match_text: one("match"),
            wait,
            settle,
            thread: one("thread"),
            thread_delay: Duration::from_secs(
                one("thread-delay")
                    .map(|secs| secs.parse().context("--thread-delay 要是秒数"))
                    .transpose()?
                    .unwrap_or(12),
            ),
            print_event_id: one("print-event-id").is_some(),
            mention: one("mention")
                .map(|mention| mention.parse())
                .transpose()
                .context("--mention 不是合法的用户 ID")?,
            redacted: one("redacted")
                .map(|secs| secs.parse().context("--redacted 要是秒数"))
                .transpose()?
                .map(Duration::from_secs),
            token_only: one("token-only").is_some(),
        })
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
    if options.token_only {
        // 不注销：调用方要拿这个令牌接着调接口。
        let token = client.access_token().context("登录后没有访问令牌")?;
        println!("令牌 {token}");
        return Ok(());
    }
    // 设备刚建好时服务器偶尔还回 404 说它不存在，等它落盘再开始收发（实测 404 出现在登录后几毫秒）。
    tokio::time::sleep(Duration::from_secs(2)).await;
    let result = converse(&client, &options).await;
    if let Err(e) = client.matrix_auth().logout().await {
        tracing::warn!(error = %e, "冒烟设备退出登录失败");
    }
    result
}

/// Bot 发来的文字（`m.text` 或 `m.notice`）：正文和是不是加密消息。
type Message = (String, bool);

/// 按顺序发每条消息，每发一条等 Bot 回一条。
async fn converse(client: &Client, options: &Options) -> anyhow::Result<()> {
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
    // 新设备的密钥要真的上服务器，对方才收得到这台设备的消息：先等这个设备的密钥传完，再问一次对方的设备表。
    client
        .encryption()
        .wait_for_e2ee_initialization_tasks()
        .await;
    ensure_recipient_devices(client, &options.bot).await?;
    let (sender, mut replies) = mpsc::unbounded_channel::<Message>();
    let (room_id, bot) = (options.room.clone(), options.bot.clone());
    let thread = Arc::new(Mutex::new(thread_root(options)?));
    let filter_thread = thread.clone();
    client.add_event_handler(
        move |event: OriginalSyncRoomMessageEvent,
              room: Room,
              encryption: Option<EncryptionInfo>| {
            let sender = sender.clone();
            let root = filter_thread.lock().unwrap().clone();
            let in_thread = match (&root, event.content.relates_to.as_ref()) {
                (None, _) => true,
                (Some(root), Some(Relation::Thread(thread))) => &thread.event_id == root,
                (Some(_), _) => false,
            };
            let from_bot = room.room_id() == room_id && event.sender == bot && in_thread;
            async move {
                let body = match event.content.msgtype {
                    MessageType::Text(text) => text.body,
                    MessageType::Notice(notice) => notice.body,
                    _ => return,
                };
                if from_bot {
                    let _ = sender.send((body, encryption.is_some()));
                }
            }
        },
    );
    // 同步和对话在同一个任务里并行：matrix-sdk 的同步 future 太深，放进 tokio::spawn 编译器证不出它是 Send。
    let settings = SyncSettings::default().token(first.next_batch);
    let sync = client.sync(settings);
    tokio::pin!(sync);
    let rounds = async {
        for (round, text) in options.texts.iter().enumerate() {
            send_round(
                client,
                &room,
                options,
                &thread,
                &mut replies,
                round + 1,
                text,
            )
            .await?;
        }
        Ok::<_, anyhow::Error>(())
    };
    tokio::select! {
        stopped = &mut sync => bail!("同步中断：{stopped:?}"),
        result = rounds => result,
    }
}

/// 这次运行里的话题根：`--thread <事件 ID>` 直接给，`auto` 时等第一条消息发出后再填。
fn thread_root(options: &Options) -> anyhow::Result<Option<OwnedEventId>> {
    options
        .thread
        .as_deref()
        .filter(|root| *root != "auto")
        .map(|root| {
            root.parse::<OwnedEventId>()
                .context("--thread 不是合法的事件 ID")
        })
        .transpose()
}

/// 以服务器为准（`/keys/query`）确认对方有上传过的设备：本地 crypto store 还没查到对方设备时，发送会
/// 一直等。顺带把对方的设备表拉到本地。
async fn ensure_recipient_devices(client: &Client, user: &OwnedUserId) -> anyhow::Result<()> {
    let mut request = get_keys::v3::Request::new();
    request.device_keys.insert(user.to_owned(), Vec::new());
    let response = client.send(request).await.context("查对方的设备密钥")?;
    let known = response
        .device_keys
        .get(user)
        .is_some_and(|devices| !devices.is_empty());
    if !known {
        bail!("{user} 在服务器上没有上传过设备密钥，发不了加密消息");
    }
    Ok(())
}

/// 发一轮：发这条消息、打印事件 ID，`auto` 的第一条当话题根，然后等 Bot 的回声。
async fn send_round(
    client: &Client,
    room: &Room,
    options: &Options,
    thread: &Arc<Mutex<Option<OwnedEventId>>>,
    replies: &mut mpsc::UnboundedReceiver<Message>,
    round: usize,
    text: &str,
) -> anyhow::Result<()> {
    let mut content = RoomMessageEventContent::text_plain(text);
    if let Some(mention) = &options.mention {
        content.mentions = Some(Mentions::with_user_ids([mention.clone()]));
    }
    if let Some(root) = thread.lock().unwrap().clone() {
        // 和桥自己的发送一样走 make_for_thread：带 m.in_reply_to 回退，服务器更认。
        let relation = Thread::plain(root.clone(), root.clone());
        let own = client.user_id().expect("登录后一定有用户 ID");
        content = content.make_for_thread(
            ReplyMetadata::new(&root, own, Some(&relation)),
            ReplyWithinThread::No,
            AddMentions::No,
        );
    }
    let event_id = room.send(content).await.context("发消息")?;
    if options.print_event_id || options.thread.as_deref() == Some("auto") {
        println!("第 {round} 轮：事件 {}", event_id.response.event_id);
    }
    // auto：这一条是话题根，先把它记下来、等一会儿，再发话题里的下一条。
    if options.thread.as_deref() == Some("auto") && thread.lock().unwrap().is_none() {
        *thread.lock().unwrap() = Some(event_id.response.event_id.clone());
        tokio::time::sleep(options.thread_delay).await;
        return Ok(());
    }
    if let Some(seconds) = options.redacted {
        verify_redacted(client, &options.room, &event_id.response.event_id, seconds).await?;
    }
    let wait = options.wait;
    let (body, encrypted) = tokio::time::timeout(
        wait,
        next_reply(replies, options.reply, text, options.match_text.as_deref()),
    )
    .await
    .ok()
    .flatten()
    .with_context(|| {
        format!(
            "第 {round} 轮：{} 秒内没收到 {} 的回复（Bot 没在运行，或者解不开这台设备的加密消息）",
            wait.as_secs(),
            options.bot
        )
    })?;
    if !encrypted {
        bail!("第 {round} 轮：回复没有加密");
    }
    println!("第 {round} 轮：收到加密的回复：{body}");
    if let Some(settle) = options.settle {
        while let Ok(Some((body, encrypted))) = tokio::time::timeout(settle, replies.recv()).await {
            if !encrypted {
                bail!("第 {round} 轮：回复没有加密");
            }
            println!("第 {round} 轮：又收到加密的回复：{body}");
        }
    }
    Ok(())
}

/// 等服务器把这条事件撤回：撤回后 content 是空对象。
async fn verify_redacted(
    client: &Client,
    room_id: &OwnedRoomId,
    event_id: &OwnedEventId,
    timeout: Duration,
) -> anyhow::Result<()> {
    let deadline = std::time::Instant::now() + timeout;
    loop {
        let request = get_room_event::v3::Request::new(room_id.clone(), event_id.clone());
        let response = client.send(request).await.context("查撤回后的事件")?;
        let value = serde_json::to_value(&response.event).context("解析事件")?;
        let content = value.get("content").cloned().unwrap_or_default();
        if content
            .as_object()
            .is_some_and(|content| content.is_empty())
        {
            println!("已撤回：content={{}}");
            return Ok(());
        }
        if std::time::Instant::now() >= deadline {
            bail!("等到超时也没撤回，content={content}");
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

/// 下一条符合要求的回复：回声模式要内容对得上，否则要有文字。
async fn next_reply(
    replies: &mut mpsc::UnboundedReceiver<Message>,
    reply: Reply,
    text: &str,
    match_text: Option<&str>,
) -> Option<Message> {
    let expected = format!("{PREFIX}{text}");
    while let Some((body, encrypted)) = replies.recv().await {
        let matches = match reply {
            Reply::Echo => body == expected,
            Reply::Any => !body.trim().is_empty(),
        } && match_text.is_none_or(|needle| body.contains(needle));
        if matches {
            return Some((body, encrypted));
        }
    }
    None
}
