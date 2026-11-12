//! 传完文件发链接：App 重启后接着发，链接照发出去，不用主人点重发；Matrix 出问题时传输在期限内停到
//! 「已传完，链接没发出」，不再一直显示在传。

use std::time::Duration;

use wonton_client::{Action, TimelineContent, Transfer, TransferState, UploadSource};
use wonton_protocol::{ChatAccount, ChatSync};

use super::accept_the_bots_invite;
use super::support::{Home, Phone, Timeline, WAIT};
use wonton_matrix::MatrixSession;

/// 一 KB 多的字节：字节一步传完，链接的发送赶在会话恢复和房间同步前面。内容只比对长度和编号。
const TINY: usize = 1200;

struct Room {
    phone: Phone,
    home: Home,
    _bot: MatrixSession,
    room: String,
    dir: tempfile::TempDir,
}

impl Room {
    async fn open() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let phone = Phone::open(dir.path());
        let home = Home::start(&[phone.device_id()]).await;
        phone.connect(&home);
        phone.signed_in().await;
        let bot = home.bot("hermes", "Hermes").await;
        let room_id = accept_the_bots_invite(&home, &phone, &bot).await;
        Self {
            phone,
            home,
            _bot: bot,
            room: room_id.to_string(),
            dir,
        }
    }

    fn write_file(&self, name: &str) -> std::path::PathBuf {
        let path = self.dir.path().join(name);
        std::fs::write(&path, vec![0xA5; TINY]).unwrap();
        path
    }

    fn upload(&self, path: &std::path::Path) -> Action {
        Action::UploadFile {
            source: UploadSource::Path {
                path: path.to_str().unwrap().to_owned(),
            },
            name: name_of(path),
            mime_type: Some("text/plain".to_owned()),
            room: Some(self.room.clone()),
        }
    }

    /// App 被杀再打开：设备身份还在，配对还有效。旧核心先完全停下再开新的，和真的重启（换进程）一样。
    async fn restart(&mut self) {
        self.phone.test_shut_down().await;
        let fresh = Phone::open(self.dir.path());
        drop(std::mem::replace(&mut self.phone, fresh));
        self.phone.connect(&self.home);
    }
}

fn name_of(path: &std::path::Path) -> String {
    path.file_name().unwrap().to_string_lossy().into_owned()
}

/// 等传输列表里有一条满足条件；默认等 [`WAIT`]，期限由调用方给。
async fn wait_transfer(phone: &Phone, what: &str, done: impl Fn(&Transfer) -> bool) -> Transfer {
    wait_transfer_within(phone, what, WAIT, done).await
}

async fn wait_transfer_within(
    phone: &Phone,
    what: &str,
    cap: Duration,
    done: impl Fn(&Transfer) -> bool,
) -> Transfer {
    let subscription = phone.subscribe_transfers();
    let waiting = async {
        while let Some(view) = subscription.next().await {
            if let Some(transfer) = view.all().find(|transfer| done(transfer)) {
                return transfer.clone();
            }
        }
        panic!("订阅提前结束");
    };
    tokio::time::timeout(cap, waiting)
        .await
        .unwrap_or_else(|_| {
            panic!(
                "{what}：{cap:?} 内没等到，传输列表：{:?}",
                phone.transfers()
            )
        })
}

/// 链接进了房间：认出文件的消息。正文里的 `wonton://file/` 核心剥掉了，文件编号放在 `files` 里。
fn link_in_room(item: &wonton_client::TimelineItem) -> Option<()> {
    match &item.content {
        TimelineContent::Message { message } => match &message.body {
            wonton_client::MessageContent::Text { files, .. } if !files.is_empty() => Some(()),
            _ => None,
        },
        _ => None,
    }
}

/// App 重启后接着发：会话恢复、房间同步都要一会儿，传输等到它们就绪，链接照发出去，不用点重发。
#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn the_link_goes_out_after_a_restart_without_resending() {
    let mut room = Room::open().await;
    let source = room.write_file("笔记.txt");
    room.phone.dispatch(room.upload(&source)).unwrap();

    // 传输刚开工就杀掉 App：字节传到哪算哪，重启后接着传、接着发。
    room.restart().await;

    // 先按状态等登录恢复（前缀条件，耗时多少都行——满载的机器上重连退避能吃掉半分钟）；
    // 登录起来之后，链接该在传输的等待窗内自己发出去。
    room.phone
        .wait_chat_within(
            "重启后登录并开始同步",
            Duration::from_secs(180),
            |view| {
                matches!(
                    (&view.account, &view.sync),
                    (ChatAccount::SignedIn { .. }, ChatSync::Running)
                )
                .then_some(())
            },
        )
        .await;

    let done = wait_transfer(&room.phone, "重启后传输完成", |transfer| {
        transfer.state == TransferState::Done
    })
    .await;
    assert_eq!(done.transferred_bytes, TINY as u64);

    let mut timeline = Timeline::open(&room.phone, &room.room).await;
    timeline.until("链接发进房间", link_in_room).await;
}

/// Matrix 停着时发不出去：传输在期限内停到「已传完，链接没发出」，恢复了点重发就发出去。期限是发送的 60 秒，
/// 恢复后 SDK 的发送队列还要按自己的退避把放弃的那条排完，重发的等在后面，多给些余量。
#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn a_link_that_cannot_go_out_stops_within_a_minute() {
    let room = Room::open().await;
    let mut timeline = Timeline::open(&room.phone, &room.room).await;

    room.home.fail_sends(true);
    let source = room.write_file("笔记.txt");
    room.phone.dispatch(room.upload(&source)).unwrap();
    let stuck = wait_transfer_within(
        &room.phone,
        "传输停到链接没发出",
        Duration::from_secs(150),
        |transfer| matches!(transfer.state, TransferState::LinkNotSent { .. }),
    )
    .await;

    room.home.fail_sends(false);
    room.phone
        .dispatch(Action::ResendFileLink {
            id: stuck.id.clone(),
        })
        .unwrap();
    let done = wait_transfer_within(
        &room.phone,
        "重发后传输完成",
        Duration::from_secs(240),
        |transfer| transfer.id == stuck.id && transfer.state == TransferState::Done,
    )
    .await;
    assert_eq!(done.file, stuck.file, "重发不重传字节：还是同一个文件");
    timeline.until("重发的链接进了房间", link_in_room).await;
}
