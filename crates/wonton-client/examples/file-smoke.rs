//! 文件的端到端冒烟（ADR-0020）：只经导出给原生壳的接口扮演一台手机，和真实的 Bot 收发文件。Bot 用真实模型，要等模型回答。
//!
//! ```bash
//! WONTON_SMOKE_PAIRING_CODE="$(deploy/dev/core/dev.sh pair | tail -n 1)" \
//!   cargo run -p wonton-client --example file-smoke -- <空的数据目录> @hermes:wonton.localhost
//! ```
//!
//! 主人的共享文件夹里要有 `documents/税务/2025-报税表.txt`（内容含“应退税 1234 元”）和名字含“全家福”的图片；要装好文件搜索
//! 插件。结果在标准输出，
//! 失败时退出码不是 0。Bot 有没有调 `wonton__view_file` 看核心的审计。

// 同 lib.rs：matrix-sdk 的 future 嵌套很深。
#![recursion_limit = "256"]

use std::collections::HashSet;
use std::sync::Arc;
use std::time::{Duration, Instant};

use wonton_client::{
    Action, ChatAccount, ClientCore, FileCard, FileId, FileInfo, FileSource, HomeStatus,
    MessageContent, SecretString, TimelineContent, TimelineDiff, TimelineItem, TimelineMessage,
    TimelineSubscription, TransferDirection, TransferState, UploadSource,
};

/// 真实模型要想一会儿，还可能先说一句再调工具。
const BOT_ROUND: Duration = Duration::from_secs(240);
const SETUP: Duration = Duration::from_secs(120);
const TRANSFER: Duration = Duration::from_secs(120);

type Outcome<T> = Result<T, String>;

fn main() {
    let mut args = std::env::args().skip(1);
    let (Some(data_dir), Some(bot)) = (args.next(), args.next()) else {
        eprintln!("用法：file-smoke <空的数据目录> <Bot 的 Matrix 用户 ID>");
        std::process::exit(2);
    };
    let code = std::env::var("WONTON_SMOKE_PAIRING_CODE").expect("要设 WONTON_SMOKE_PAIRING_CODE");
    let core = ClientCore::new(data_dir.clone()).expect("核心起不来");
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("建不了运行时");
    let result = runtime.block_on(run(&core, &code, &bot, &data_dir));
    let room = core
        .chat()
        .rooms
        .iter()
        .find(|row| row.direct_user_id.as_deref() == Some(bot.as_str()))
        .map(|row| row.room_id.clone());
    drop(core);
    let result = result.and_then(|()| {
        let room = room.ok_or("私聊不见了")?;
        runtime.block_on(resume_after_restart(&data_dir, &room))
    });
    match result {
        Ok(()) => println!("全部通过"),
        Err(failure) => {
            println!("✗ {failure}");
            std::process::exit(1);
        }
    }
}

async fn run(core: &Arc<ClientCore>, code: &str, bot: &str, data_dir: &str) -> Outcome<()> {
    let room = connect(core, code, bot).await?;
    let mut chat = Chat::open(core, &room, bot).await?;
    println!("✓ 配对、登录，找到和 {bot} 的私聊 {room}");

    let shared = chat
        .ask_for_file(
            "在文件搜索里找名字含“2025-报税表”的文件，用 wonton__share_file 发给我。只发文件，不用读内容。",
        )
        .await?;
    println!("✓ Bot 分享的文件进了消息：{shared}");
    let info = card(core, &shared).await?;
    println!(
        "✓ 卡片：{}，{:?} 字节，来源 {:?}",
        info.name, info.size, info.source
    );
    if !matches!(info.source, FileSource::Bot { .. }) {
        return Err(format!("卡片应写明来自 Bot：{:?}", info.source));
    }
    let bytes = download(core, &info).await?;
    let text = String::from_utf8_lossy(&bytes);
    if !text.contains("应退税 1234 元") {
        return Err(format!("下载到的内容不对：{text}"));
    }
    println!("✓ 下载成功，内容对得上");
    play(core, &shared, &bytes).await?;
    println!("✓ 播放地址支持 Range，给出同样的字节");

    let uploaded = upload(core, data_dir, &room).await?;
    println!("✓ 上传做完，编号 {uploaded}");
    chat.wait_own_link(&uploaded).await?;
    println!("✓ 传完的链接由核心发进了房间，卡片认得出");
    let answer = chat
        .ask(
            "我刚发给你一张图。用 wonton__view_file 看一下，告诉我它主要是什么颜色，一句话。没看到图就直接说看不了。",
        )
        .await?;
    println!("✓ Bot 看图后的回答：{answer}");

    let photo = chat
        .ask_for_file(
            "在文件搜索里找名字含“全家福”的图片，看一眼，然后用 wonton__share_file 发给我。",
        )
        .await?;
    let info = card(core, &photo).await?;
    let thumbnail = core
        .file_thumbnail(photo.clone())
        .await
        .map_err(|e| format!("取不到缩略图：{e}"))?;
    println!(
        "✓ Bot 看过再分享的图片也是卡片：{}，有缩略图 {}，缩略图 {} 字节",
        info.name,
        info.has_thumbnail,
        thumbnail.len()
    );
    Ok(())
}

/// 大文件传到一半 App 被杀，重启后接着传，经真实的隧道、tusd 和核心。
async fn resume_after_restart(data_dir: &str, room: &str) -> Outcome<()> {
    const SIZE: usize = 128 * 1024 * 1024;
    let bytes: Vec<u8> = (0..SIZE).map(|i| (i % 251) as u8).collect();
    let path = std::path::Path::new(data_dir).join("大文件.bin");
    std::fs::write(&path, &bytes).map_err(|e| e.to_string())?;

    let core = ClientCore::new(data_dir.to_owned()).map_err(|e| e.to_string())?;
    core.dispatch(Action::UploadFile {
        source: UploadSource::Path {
            path: path.to_string_lossy().into_owned(),
        },
        name: "大文件.bin".to_owned(),
        mime_type: None,
        room: Some(room.to_owned()),
    })
    .map_err(|e| format!("上传被拒绝：{e}"))?;
    let upload_at = kill_midway(core, TransferDirection::Upload).await?;
    let core = ClientCore::new(data_dir.to_owned()).map_err(|e| e.to_string())?;
    let transfer = wait_named(&core, "大文件.bin", TransferDirection::Upload).await?;
    let file = transfer.file.ok_or("做完的上传没有编号")?;
    println!("✓ 上传到 {upload_at} 字节时杀掉，重启后接着传完");

    core.dispatch(Action::DownloadFile { id: file })
        .map_err(|e| format!("下载被拒绝：{e}"))?;
    let download_at = kill_midway(core, TransferDirection::Download).await?;
    let core = ClientCore::new(data_dir.to_owned()).map_err(|e| e.to_string())?;
    let transfer = wait_named(&core, "大文件.bin", TransferDirection::Download).await?;
    let destination = transfer.destination.ok_or("做完的下载没有路径")?;
    let got = std::fs::read(&destination).map_err(|e| e.to_string())?;
    if got != bytes {
        return Err("续传下载到的内容和上传的不一样".to_owned());
    }
    println!("✓ 下载到 {download_at} 字节时杀掉，重启后接着下完，内容逐字节相同");
    tokio::task::spawn_blocking(move || drop(core))
        .await
        .map_err(|e| e.to_string())
}

/// 等到这个方向的传输动了但没传完就释放核心，返回当时的进度。
async fn kill_midway(core: Arc<ClientCore>, direction: TransferDirection) -> Outcome<u64> {
    let deadline = Instant::now() + TRANSFER;
    let at = loop {
        let list = core.transfers();
        let moving = list
            .all()
            .find(|t| t.direction == direction && t.name == "大文件.bin");
        if let Some(transfer) = moving {
            if transfer.state == TransferState::Done {
                return Err("传得太快，没来得及杀掉：换更大的文件".to_owned());
            }
            if transfer.transferred_bytes > 0 {
                break transfer.transferred_bytes;
            }
        }
        if Instant::now() > deadline {
            return Err(format!("传输没动：{list:?}"));
        }
        tokio::time::sleep(Duration::from_millis(2)).await;
    };
    tokio::task::spawn_blocking(move || drop(core))
        .await
        .map_err(|e| e.to_string())?;
    Ok(at)
}

async fn wait_named(
    core: &Arc<ClientCore>,
    name: &str,
    direction: TransferDirection,
) -> Outcome<wonton_client::Transfer> {
    let deadline = Instant::now() + TRANSFER;
    loop {
        let list = core.transfers();
        if let Some(transfer) = list
            .all()
            .find(|t| t.direction == direction && t.name == name)
        {
            match &transfer.state {
                TransferState::Done => return Ok(transfer.clone()),
                TransferState::Failed { reason, message } => {
                    return Err(format!("传输失败：{reason:?}：{message}"));
                }
                TransferState::LinkNotSent { reason, message } => {
                    return Err(format!("链接没发出：{reason:?}：{message}"));
                }
                TransferState::Running | TransferState::Paused => {}
            }
        }
        if Instant::now() > deadline {
            return Err(format!("传输超时：{list:?}"));
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

async fn connect(core: &Arc<ClientCore>, code: &str, bot: &str) -> Outcome<String> {
    core.dispatch(Action::PairHome {
        code: SecretString::new(code),
        device_name: "file-smoke".to_owned(),
    })
    .map_err(|e| format!("配对被拒绝：{e}"))?;
    let deadline = Instant::now() + SETUP;
    loop {
        let home = core.home();
        let chat = core.chat();
        if let HomeStatus::Disconnected { message, .. } = &home.status {
            println!("  连接：{message}");
        }
        let signed_in = matches!(chat.account, ChatAccount::SignedIn { .. });
        let room = chat
            .rooms
            .iter()
            .find(|row| row.direct_user_id.as_deref() == Some(bot));
        if let (true, Some(room)) = (signed_in, room) {
            return Ok(room.room_id.clone());
        }
        if Instant::now() > deadline {
            return Err(format!("等不到私聊：{:?}，{:?}", home.status, chat.account));
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

struct Chat<'a> {
    core: &'a Arc<ClientCore>,
    room: String,
    bot: String,
    timeline: Arc<TimelineSubscription>,
    /// 开始问之前就有的 Bot 消息不算回答。
    seen: HashSet<String>,
}

impl<'a> Chat<'a> {
    async fn open(core: &'a Arc<ClientCore>, room: &str, bot: &str) -> Outcome<Self> {
        let deadline = Instant::now() + SETUP;
        let timeline = loop {
            match core.subscribe_timeline(room.to_owned()).await {
                Ok(timeline) => break timeline,
                Err(error) if Instant::now() > deadline => {
                    return Err(format!("打不开时间线：{error}"));
                }
                Err(_) => tokio::time::sleep(Duration::from_millis(500)).await,
            }
        };
        let mut chat = Self {
            core,
            room: room.to_owned(),
            bot: bot.to_owned(),
            timeline,
            seen: HashSet::new(),
        };
        // 第一批是已有的全部条目。
        chat.next_messages(Duration::from_secs(10)).await?;
        Ok(chat)
    }

    async fn next_messages(&mut self, wait: Duration) -> Outcome<Vec<TimelineMessage>> {
        let update = tokio::time::timeout(wait, self.timeline.next())
            .await
            .map_err(|_| "等消息超时".to_owned())?
            .ok_or("时间线关了")?;
        let messages: Vec<_> = update.diffs.iter().flat_map(messages_in).collect();
        Ok(messages)
    }

    fn send(&self, text: &str) -> Outcome<()> {
        println!("  → {text}");
        self.core
            .dispatch(Action::SendText {
                room_id: self.room.clone(),
                body: text.to_owned(),
                reply_to: None,
            })
            .map_err(|e| format!("发不出去：{e}"))
    }

    /// 等 Bot 的消息，直到 `done` 有结果。流式回复靠编辑消息，同一条会来好几次。
    async fn wait_bot<T>(
        &mut self,
        mut done: impl FnMut(&TimelineMessage) -> Option<T>,
    ) -> Outcome<T> {
        let deadline = Instant::now() + BOT_ROUND;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err("等 Bot 回答超时".to_owned());
            }
            for message in self.next_messages(left).await? {
                let id = message.event_id.clone().unwrap_or_default();
                if message.sender_id == self.bot && !self.seen.contains(&id) {
                    println!("  ← {:?}", message.body);
                    if let Some(found) = done(&message) {
                        self.seen.insert(id);
                        return Ok(found);
                    }
                }
            }
        }
    }

    async fn ask_for_file(&mut self, text: &str) -> Outcome<FileId> {
        self.send(text)?;
        self.wait_bot(|message| match &message.body {
            MessageContent::Text { files, .. } => files.first().cloned(),
            _ => None,
        })
        .await
    }

    /// Bot 的回答：回答前可能先说一句“我看看”，等它停下来 20 秒，取最后说的。
    async fn ask(&mut self, text: &str) -> Outcome<String> {
        self.send(text)?;
        let mut answer = self
            .wait_bot(|message| match &message.body {
                MessageContent::Text { body, .. } if !body.is_empty() => Some(body.clone()),
                _ => None,
            })
            .await?;
        while let Ok(messages) = self.next_messages(Duration::from_secs(20)).await {
            for message in messages {
                if message.sender_id == self.bot
                    && let MessageContent::Text { body, .. } = message.body
                    && !body.is_empty()
                {
                    println!("  ← {body}");
                    answer = body;
                }
            }
        }
        Ok(answer)
    }

    async fn wait_own_link(&mut self, file: &FileId) -> Outcome<()> {
        let deadline = Instant::now() + TRANSFER;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err("等不到发进房间的链接".to_owned());
            }
            for message in self.next_messages(left).await? {
                if message.is_own
                    && let MessageContent::Text { files, .. } = &message.body
                    && files.contains(file)
                {
                    return Ok(());
                }
            }
        }
    }
}

fn messages_in(diff: &TimelineDiff) -> Vec<TimelineMessage> {
    let items: Vec<&TimelineItem> = match diff {
        TimelineDiff::Append { items } | TimelineDiff::Reset { items } => items.iter().collect(),
        TimelineDiff::PushFront { item }
        | TimelineDiff::PushBack { item }
        | TimelineDiff::Insert { item, .. }
        | TimelineDiff::Set { item, .. } => vec![item],
        _ => Vec::new(),
    };
    items
        .into_iter()
        .filter_map(|item| match &item.content {
            TimelineContent::Message { message } => Some(message.clone()),
            _ => None,
        })
        .collect()
}

async fn card(core: &Arc<ClientCore>, id: &FileId) -> Outcome<FileInfo> {
    match core.file_card(id.clone()).await {
        FileCard::Ready { info } => Ok(info),
        other => Err(format!("卡片看不了：{other:?}")),
    }
}

async fn download(core: &Arc<ClientCore>, info: &FileInfo) -> Outcome<Vec<u8>> {
    core.dispatch(Action::DownloadFile {
        id: info.id.clone(),
    })
    .map_err(|e| format!("下载被拒绝：{e}"))?;
    let transfer = wait_transfer(core, TransferDirection::Download).await?;
    let path = transfer.destination.ok_or("做完的下载没有路径")?;
    std::fs::read(&path).map_err(|e| format!("读不了 {path}：{e}"))
}

async fn wait_transfer(
    core: &Arc<ClientCore>,
    direction: TransferDirection,
) -> Outcome<wonton_client::Transfer> {
    let deadline = Instant::now() + TRANSFER;
    loop {
        let list = core.transfers();
        if let Some(transfer) = list.all().find(|t| t.direction == direction) {
            match &transfer.state {
                TransferState::Done => return Ok(transfer.clone()),
                TransferState::Failed { reason, message } => {
                    return Err(format!("传输失败：{reason:?}：{message}"));
                }
                TransferState::LinkNotSent { reason, message } => {
                    return Err(format!("链接没发出：{reason:?}：{message}"));
                }
                TransferState::Running | TransferState::Paused => {}
            }
        }
        if Instant::now() > deadline {
            return Err(format!("传输超时：{list:?}"));
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

async fn play(core: &Arc<ClientCore>, id: &FileId, bytes: &[u8]) -> Outcome<()> {
    let url = core
        .file_stream_url(id.clone())
        .await
        .map_err(|e| format!("拿不到播放地址：{e}"))?;
    // 本地的回环地址，不经系统代理。
    let client = reqwest::Client::builder()
        .no_proxy()
        .build()
        .map_err(|e| e.to_string())?;
    let response = client
        .get(&url)
        .header("Range", "bytes=1-")
        .send()
        .await
        .map_err(|e| format!("播放地址连不上：{e}"))?;
    if response.status() != 206 {
        return Err(format!("播放地址应回 206：{}", response.status()));
    }
    let got = response.bytes().await.map_err(|e| e.to_string())?;
    if got.as_ref() != &bytes[1..] {
        return Err("播放地址给的字节不对".to_owned());
    }
    Ok(())
}

async fn upload(core: &Arc<ClientCore>, data_dir: &str, room: &str) -> Outcome<FileId> {
    let path = std::path::Path::new(data_dir).join("测试图.png");
    std::fs::write(&path, green_png()).map_err(|e| e.to_string())?;
    core.dispatch(Action::UploadFile {
        source: UploadSource::Path {
            path: path.to_string_lossy().into_owned(),
        },
        name: "测试图.png".to_owned(),
        mime_type: Some("image/png".to_owned()),
        room: Some(room.to_owned()),
    })
    .map_err(|e| format!("上传被拒绝：{e}"))?;
    let transfer = wait_transfer(core, TransferDirection::Upload).await?;
    transfer.file.ok_or_else(|| "做完的上传没有编号".to_owned())
}

/// 一张 2×2 的纯绿 PNG：像素不压缩的 deflate 块，用不着图片库。
fn green_png() -> Vec<u8> {
    const PNG: &[u8] = &[
        0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44,
        0x52, 0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x02, 0x08, 0x02, 0x00, 0x00, 0x00, 0xfd,
        0xd4, 0x9a, 0x73,
    ];
    let mut out = PNG.to_vec();
    let raw: Vec<u8> = (0..2).flat_map(|_| [0u8, 0, 255, 0, 0, 255, 0]).collect();
    let mut idat = vec![0x78, 0x01, 0x01];
    idat.extend((raw.len() as u16).to_le_bytes());
    idat.extend((!(raw.len() as u16)).to_le_bytes());
    idat.extend(&raw);
    idat.extend(adler32(&raw).to_be_bytes());
    chunk(&mut out, b"IDAT", &idat);
    chunk(&mut out, b"IEND", &[]);
    out
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend((data.len() as u32).to_be_bytes());
    let start = out.len();
    out.extend(kind);
    out.extend(data);
    let crc = crc32(&out[start..]);
    out.extend(crc.to_be_bytes());
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = !0u32;
    for byte in data {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = if crc & 1 == 1 {
                (crc >> 1) ^ 0xedb8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

fn adler32(data: &[u8]) -> u32 {
    let (mut low, mut high) = (1u32, 0u32);
    for byte in data {
        low = (low + u32::from(*byte)) % 65521;
        high = (high + low) % 65521;
    }
    (high << 16) | low
}
