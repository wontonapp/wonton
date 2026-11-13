//! 附件区的行为测试：给定一串操作，断言壳看到的 ViewModel 和交给 Matrix 的事件。Matrix 和文件服务换成测试替身。

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::future::BoxFuture;
use serde_json::json;
use tokio::sync::{oneshot, watch};
use wonton_protocol::{
    AttachmentFailure, AttachmentState, ComposerViewModel, CoreError, FileId, ImageUpload,
    SendState, Transfer, TransferDirection, TransferState, TransfersViewModel, UploadSource,
};

use super::ComposerLink;
use super::uplink::{Handed, Handover, Part, Progress, Uplink, UplinkError, Uploaded};

const ROOM: &str = "!room:example.org";

/// 交给发送队列的一组，测试替身记下来的样子。
#[derive(Debug)]
struct Group {
    parts: Vec<Part>,
    group_id: Option<String>,
    reply_to: Option<String>,
    skip: usize,
}

struct Fake {
    uploads: Mutex<HashMap<String, oneshot::Sender<Result<Uploaded, UplinkError>>>>,
    upload_calls: Mutex<Vec<String>>,
    encrypted: Mutex<bool>,
    online: watch::Sender<bool>,
    transfers: watch::Sender<TransfersViewModel>,
    /// 授权和交出按发生的顺序记一份，看授权是不是在交出之前。
    log: Mutex<Vec<String>>,
    groups: Mutex<Vec<Group>>,
    /// 交出这么多条后断线一次。
    cut_after: Mutex<Option<usize>>,
    drafts: Mutex<Vec<String>>,
}

impl Fake {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            uploads: Mutex::default(),
            upload_calls: Mutex::default(),
            encrypted: Mutex::new(false),
            online: watch::Sender::new(true),
            transfers: watch::Sender::new(TransfersViewModel::default()),
            log: Mutex::default(),
            groups: Mutex::default(),
            cut_after: Mutex::default(),
            drafts: Mutex::default(),
        })
    }

    fn upload_calls(&self) -> usize {
        self.upload_calls.lock().unwrap().len()
    }

    fn groups(&self) -> usize {
        self.groups.lock().unwrap().len()
    }

    fn finish(&self, name: &str, result: Result<Uploaded, UplinkError>) {
        let sender = self
            .uploads
            .lock()
            .unwrap()
            .remove(name)
            .expect("这张照片在传");
        sender.send(result).expect("上传任务还在等");
    }

    fn finish_file(&self, transfer: &str, file: &FileId) {
        self.transfers.send_modify(|view| {
            let transfer = view
                .active
                .iter_mut()
                .find(|seen| seen.id == transfer)
                .expect("这个文件在传");
            transfer.state = TransferState::Done;
            transfer.file = Some(file.clone());
            transfer.transferred_bytes = 100;
        });
    }
}

fn uploaded(name: &str, encrypted: bool) -> Uploaded {
    Uploaded {
        handle: json!({ "name": name, "encrypted": encrypted }),
        encrypted,
    }
}

fn image(uploaded: Uploaded, caption: Option<&str>) -> Part {
    Part::Image {
        uploaded,
        caption: caption.map(str::to_owned),
    }
}

impl Uplink for Fake {
    fn check_send(&self, _room: &str, _reply_to: Option<&str>) -> Result<(), CoreError> {
        Ok(())
    }

    fn forget_draft(&self, _room: &str, body: &str) {
        self.drafts.lock().unwrap().push(body.to_owned());
    }

    fn sent(&self, _room: &str) {}

    fn upload_image(
        &self,
        _room: String,
        image: wonton_matrix::ImageUpload,
        progress: Progress,
    ) -> BoxFuture<'static, Result<Uploaded, UplinkError>> {
        if !*self.online.borrow() {
            return Box::pin(async { Err(UplinkError::Offline("断网".into())) });
        }
        let (sender, receiver) = oneshot::channel();
        self.uploads
            .lock()
            .unwrap()
            .insert(image.filename.clone(), sender);
        self.upload_calls.lock().unwrap().push(image.filename);
        progress(1, 2);
        Box::pin(async move {
            receiver
                .await
                .unwrap_or_else(|_| Err(UplinkError::Offline("替身丢了这次上传".into())))
        })
    }

    fn room_is_encrypted(&self, _room: String) -> BoxFuture<'static, Result<bool, UplinkError>> {
        let encrypted = *self.encrypted.lock().unwrap();
        Box::pin(async move { Ok(encrypted) })
    }

    fn start_file(
        &self,
        _source: UploadSource,
        name: String,
        _mime_type: Option<String>,
    ) -> Result<String, UplinkError> {
        let id = format!("t-{name}");
        self.transfers.send_modify(|view| {
            view.active.push(Transfer {
                id: id.clone(),
                file: None,
                name,
                direction: TransferDirection::Upload,
                room: None,
                transferred_bytes: 0,
                total_bytes: Some(100),
                state: TransferState::Running,
                paused_because: None,
                destination: None,
                actions: Vec::new(),
            });
        });
        Ok(id)
    }

    fn drop_file(&self, transfer: &str) {
        self.log.lock().unwrap().push(format!("drop {transfer}"));
    }

    fn transfers(&self) -> watch::Receiver<TransfersViewModel> {
        self.transfers.subscribe()
    }

    fn grant(&self, file: FileId, _room: String) -> BoxFuture<'static, Result<(), UplinkError>> {
        self.log.lock().unwrap().push(format!("grant {file}"));
        Box::pin(async { Ok(()) })
    }

    fn send_group(
        &self,
        handover: Handover,
        handed: Handed,
    ) -> BoxFuture<'static, Result<(), UplinkError>> {
        self.log.lock().unwrap().push("send".into());
        let cut = self.cut_after.lock().unwrap().take();
        let total = handover.parts.len();
        let stop = cut.unwrap_or(total);
        for count in handover.skip + 1..=stop {
            handed(count);
        }
        self.groups.lock().unwrap().push(Group {
            parts: handover.parts,
            group_id: handover.group_id,
            reply_to: handover.reply_to,
            skip: handover.skip,
        });
        Box::pin(async move {
            match cut {
                Some(_) => Err(UplinkError::Offline("交到一半断了".into())),
                None => Ok(()),
            }
        })
    }

    fn online(&self) -> BoxFuture<'static, ()> {
        let mut online = self.online.subscribe();
        Box::pin(async move {
            let _ = online.wait_for(|online| *online).await;
        })
    }
}

fn photo(name: &str) -> ImageUpload {
    ImageUpload {
        filename: name.to_owned(),
        mime_type: "image/jpeg".into(),
        data: vec![1, 2, 3],
        width: Some(4),
        height: Some(3),
        thumbnail: None,
    }
}

fn file_source(name: &str) -> UploadSource {
    UploadSource::Path {
        path: format!("/tmp/{name}"),
    }
}

struct Setup {
    link: Arc<ComposerLink>,
    fake: Arc<Fake>,
    view: watch::Receiver<ComposerViewModel>,
    _dir: tempfile::TempDir,
}

fn setup() -> Setup {
    let dir = tempfile::tempdir().unwrap();
    let fake = Fake::new();
    let link = ComposerLink::new(tokio::runtime::Handle::current(), fake.clone(), dir.path());
    let view = link.subscribe(ROOM);
    Setup {
        link,
        fake,
        view,
        _dir: dir,
    }
}

async fn until(what: &str, mut done: impl FnMut() -> bool) {
    for _ in 0..500 {
        if done() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("等不到：{what}");
}

/// 再等一会儿，确认后台没有做不该做的事。
async fn settle() {
    tokio::time::sleep(Duration::from_millis(100)).await;
}

fn view(setup: &Setup) -> ComposerViewModel {
    setup.view.borrow().clone()
}

#[tokio::test(flavor = "multi_thread")]
async fn photos_and_text_go_out_as_one_group_only_after_every_photo_is_up() {
    let s = setup();
    s.link.add_image(ROOM, photo("a.jpg")).unwrap();
    s.link.add_image(ROOM, photo("b.jpg")).unwrap();
    until("两张都在传", || s.fake.upload_calls() == 2).await;
    assert_eq!(view(&s).attachments.len(), 2);

    s.link.send(ROOM, "哪张好看", None).unwrap();
    let pending = view(&s);
    assert!(pending.attachments.is_empty(), "附件区清空了");
    assert_eq!(pending.outgoing.len(), 1, "马上出现一条待发消息");
    assert_eq!(pending.outgoing[0].body, "哪张好看");
    assert!(matches!(
        pending.outgoing[0].state,
        SendState::Sending { .. }
    ));
    assert_eq!(*s.fake.drafts.lock().unwrap(), ["哪张好看"]);

    s.fake.finish("a.jpg", Ok(uploaded("a.jpg", true)));
    until("第一张传完", || {
        view(&s).outgoing[0].attachments[0].state == AttachmentState::Ready
    })
    .await;
    settle().await;
    assert_eq!(s.fake.groups(), 0, "第二张没传完，一条都不发");
    assert_eq!(
        view(&s).outgoing[0].state,
        SendState::Sending {
            progress: Some(0.75)
        }
    );

    s.fake.finish("b.jpg", Ok(uploaded("b.jpg", true)));
    until("交出去了", || s.fake.groups() == 1).await;
    {
        let groups = s.fake.groups.lock().unwrap();
        let group = &groups[0];
        assert_eq!(
            group.parts,
            [
                image(uploaded("a.jpg", true), None),
                image(uploaded("b.jpg", true), Some("哪张好看")),
            ]
        );
        let group_id = group.group_id.as_deref().expect("带分组标记");
        assert!(
            wonton_protocol::MessageGroup {
                id: group_id.to_owned(),
                index: 0,
                count: 2
            }
            .is_valid()
        );
    }
    until("待发消息由本地回显代替", || {
        view(&s).outgoing.is_empty()
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn files_are_granted_before_the_text_with_their_links_goes_out() {
    let s = setup();
    s.link.add_image(ROOM, photo("p.jpg")).unwrap();
    s.link
        .add_file(ROOM, file_source("report.pdf"), "report.pdf".into(), None)
        .unwrap();
    until("照片在传", || s.fake.upload_calls() == 1).await;
    until("文件在传", || {
        s.fake.transfers.borrow().active.len() == 1
    })
    .await;
    s.link
        .send(ROOM, "看看这个", Some("$event:example.org".into()))
        .unwrap();

    s.fake.finish("p.jpg", Ok(uploaded("p.jpg", true)));
    settle().await;
    assert_eq!(s.fake.groups(), 0, "文件没传完，一条都不发");

    let file = FileId::from_bytes([7; 16]);
    s.fake.finish_file("t-report.pdf", &file);
    until("交出去了", || s.fake.groups() == 1).await;
    assert_eq!(
        *s.fake.log.lock().unwrap(),
        [format!("grant {file}"), "send".to_owned()],
        "先授权，再交出"
    );
    let groups = s.fake.groups.lock().unwrap();
    assert_eq!(
        groups[0].parts,
        [
            image(uploaded("p.jpg", true), None),
            Part::Text(format!("看看这个\n\n{}", file.link())),
        ],
        "有文件时字和链接在最后一条，照片不带说明文字"
    );
    assert!(groups[0].group_id.is_some());
    assert_eq!(groups[0].reply_to.as_deref(), Some("$event:example.org"));
}

/// 附件区的文件画在附件区和待发消息里，房间的传输条不再列；别的传输照列。附件区的上传不带房间，不拿掉的话每个房间都会列。
#[tokio::test(flavor = "multi_thread")]
async fn the_transfer_strip_leaves_out_the_trays_own_uploads() {
    let s = setup();
    s.link
        .add_file(ROOM, file_source("a.pdf"), "a.pdf".into(), None)
        .unwrap();
    s.link
        .add_file(ROOM, file_source("b.pdf"), "b.pdf".into(), None)
        .unwrap();
    until("文件在传", || {
        s.fake.transfers.borrow().active.len() == 2
    })
    .await;
    settle().await;
    s.link.send(ROOM, "", None).unwrap();
    s.link
        .add_file(ROOM, file_source("c.pdf"), "c.pdf".into(), None)
        .unwrap();
    until("第三个在传", || {
        s.fake.transfers.borrow().active.len() == 3
    })
    .await;
    settle().await;
    s.fake.transfers.send_modify(|view| {
        let mut other = view.active[0].clone();
        other.id = "download".into();
        other.direction = TransferDirection::Download;
        view.active.push(other);
    });

    let listed = s
        .link
        .hide_own_uploads(s.fake.transfers.borrow().for_room(ROOM, &[]));
    let ids: Vec<_> = listed
        .here
        .iter()
        .map(|transfer| transfer.id.as_str())
        .collect();
    assert_eq!(ids, ["download"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_failed_photo_turns_the_message_red_and_retry_sends_it() {
    let s = setup();
    let id = s.link.add_image(ROOM, photo("big.jpg")).unwrap();
    s.link.send(ROOM, "", None).unwrap();
    until("在传", || s.fake.upload_calls() == 1).await;
    s.fake.finish(
        "big.jpg",
        Err(UplinkError::Failed(
            AttachmentFailure::TooLarge,
            "太大".into(),
        )),
    );
    until("整条标红", || {
        matches!(
            view(&s).outgoing.first().map(|outgoing| &outgoing.state),
            Some(SendState::Failed {
                recoverable: false,
                ..
            })
        )
    })
    .await;
    let outgoing = view(&s).outgoing[0].clone();
    assert_eq!(outgoing.attachments[0].id, id);
    assert!(matches!(
        outgoing.attachments[0].state,
        AttachmentState::Failed {
            reason: AttachmentFailure::TooLarge,
            ..
        }
    ));
    settle().await;
    assert_eq!(s.fake.groups(), 0);

    s.link.retry_outgoing(ROOM, &outgoing.id).unwrap();
    until("只重传失败的附件", || s.fake.upload_calls() == 2).await;
    assert!(matches!(
        view(&s).outgoing[0].state,
        SendState::Sending { .. }
    ));
    s.fake.finish("big.jpg", Ok(uploaded("big.jpg", true)));
    until("交出去了", || s.fake.groups() == 1).await;
    assert_eq!(
        s.fake.groups.lock().unwrap()[0].parts,
        [image(uploaded("big.jpg", true), None)],
        "没有字就没有说明文字"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn later_messages_wait_for_the_one_in_front() {
    let s = setup();
    s.link.add_image(ROOM, photo("a.jpg")).unwrap();
    s.link.send(ROOM, "第一条", None).unwrap();
    s.link.send(ROOM, "第二条", None).unwrap();
    until("在传", || s.fake.upload_calls() == 1).await;
    settle().await;
    assert_eq!(s.fake.groups(), 0, "只有字的也排在前面那条后面");
    assert_eq!(view(&s).outgoing.len(), 2);

    s.fake.finish("a.jpg", Ok(uploaded("a.jpg", true)));
    until("两条都交出去了", || s.fake.groups() == 2).await;
    let groups = s.fake.groups.lock().unwrap();
    assert_eq!(
        groups[0].parts,
        [image(uploaded("a.jpg", true), Some("第一条"))]
    );
    assert_eq!(groups[1].parts, [Part::Text("第二条".into())]);
    assert_eq!(groups[1].group_id, None, "只有字的不带分组标记");
}

#[tokio::test(flavor = "multi_thread")]
async fn cancelling_drops_the_whole_message_and_lets_the_next_go() {
    let s = setup();
    s.link.add_image(ROOM, photo("a.jpg")).unwrap();
    s.link
        .add_file(ROOM, file_source("b.pdf"), "b.pdf".into(), None)
        .unwrap();
    s.link.send(ROOM, "不要了", None).unwrap();
    s.link.send(ROOM, "这条要", None).unwrap();
    until("在传", || s.fake.upload_calls() == 1).await;
    until("文件在传", || {
        s.fake.transfers.borrow().active.len() == 1
    })
    .await;
    settle().await;

    let first = view(&s).outgoing[0].id.clone();
    s.link.cancel_outgoing(ROOM, &first).unwrap();
    until("后面那条交出去了", || s.fake.groups() == 1).await;
    assert_eq!(
        s.fake.groups.lock().unwrap()[0].parts,
        [Part::Text("这条要".into())]
    );
    assert!(
        s.fake
            .log
            .lock()
            .unwrap()
            .contains(&"drop t-b.pdf".to_owned()),
        "文件的上传停下了"
    );
    until("待发消息清空", || view(&s).outgoing.is_empty()).await;
    assert!(matches!(
        s.link.cancel_outgoing(ROOM, &first),
        Err(CoreError::InvalidInput { .. })
    ));
}

#[tokio::test(flavor = "multi_thread")]
async fn uploads_pause_offline_and_resume_when_back() {
    let s = setup();
    s.fake.online.send_replace(false);
    s.link.add_image(ROOM, photo("a.jpg")).unwrap();
    until("暂停", || {
        view(&s).attachments[0].state == AttachmentState::Paused
    })
    .await;

    s.fake.online.send_replace(true);
    until("连上后接着传", || s.fake.upload_calls() == 1).await;
    until("显示进度", || {
        view(&s).attachments[0].state
            == AttachmentState::Uploading {
                progress: Some(0.5),
            }
    })
    .await;
    s.fake.finish("a.jpg", Ok(uploaded("a.jpg", true)));
    until("传完", || {
        view(&s).attachments[0].state == AttachmentState::Ready
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn the_tray_and_pending_messages_survive_a_restart() {
    let s = setup();
    s.link.add_image(ROOM, photo("a.jpg")).unwrap();
    s.link.send(ROOM, "重启后发", None).unwrap();
    s.link.add_image(ROOM, photo("b.jpg")).unwrap();
    until("在传", || s.fake.upload_calls() == 2).await;

    // App 被杀：换一个核心读同一个目录。
    let fake = Fake::new();
    let link = ComposerLink::new(
        tokio::runtime::Handle::current(),
        fake.clone(),
        s._dir.path(),
    );
    let restored = link.subscribe(ROOM).borrow().clone();
    assert_eq!(restored.outgoing.len(), 1);
    assert_eq!(restored.outgoing[0].body, "重启后发");
    assert_eq!(restored.attachments.len(), 1, "附件区也还在");
    until("照片从头重传", || fake.upload_calls() == 2).await;
    fake.finish("a.jpg", Ok(uploaded("a.jpg", true)));
    until("接着发", || fake.groups() == 1).await;
    assert_eq!(
        fake.groups.lock().unwrap()[0].parts,
        [image(uploaded("a.jpg", true), Some("重启后发"))]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_plain_photo_is_uploaded_again_when_the_room_became_encrypted() {
    let s = setup();
    s.link.add_image(ROOM, photo("a.jpg")).unwrap();
    until("在传", || s.fake.upload_calls() == 1).await;
    s.fake.finish("a.jpg", Ok(uploaded("a.jpg", false)));
    until("传完", || {
        view(&s).attachments[0].state == AttachmentState::Ready
    })
    .await;

    *s.fake.encrypted.lock().unwrap() = true;
    s.link.send(ROOM, "", None).unwrap();
    until("加密重传", || s.fake.upload_calls() == 2).await;
    settle().await;
    assert_eq!(s.fake.groups(), 0, "不把明文照片发进加密房间");
    s.fake.finish("a.jpg", Ok(uploaded("a.jpg", true)));
    until("交出去了", || s.fake.groups() == 1).await;
    assert_eq!(
        s.fake.groups.lock().unwrap()[0].parts,
        [image(uploaded("a.jpg", true), None)]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_handover_cut_short_continues_where_it_stopped() {
    let s = setup();
    s.link.add_image(ROOM, photo("a.jpg")).unwrap();
    s.link.add_image(ROOM, photo("b.jpg")).unwrap();
    until("在传", || s.fake.upload_calls() == 2).await;
    *s.fake.cut_after.lock().unwrap() = Some(1);
    s.link.send(ROOM, "两张", None).unwrap();
    s.fake.finish("a.jpg", Ok(uploaded("a.jpg", true)));
    s.fake.finish("b.jpg", Ok(uploaded("b.jpg", true)));
    until("交了两次", || s.fake.groups() == 2).await;
    let groups = s.fake.groups.lock().unwrap();
    assert_eq!(groups[0].skip, 0);
    assert_eq!(groups[1].skip, 1, "交出去的那条不再交一次");
    assert_eq!(groups[0].group_id, groups[1].group_id, "还是同一组");
}

#[tokio::test(flavor = "multi_thread")]
async fn sending_and_adding_refuse_what_cannot_go_out() {
    let s = setup();
    assert!(matches!(
        s.link.send(ROOM, "  ", None),
        Err(CoreError::InvalidInput { .. })
    ));
    let mut not_image = photo("a.pdf");
    not_image.mime_type = "application/pdf".into();
    assert!(matches!(
        s.link.add_image(ROOM, not_image),
        Err(CoreError::InvalidInput { .. })
    ));
    assert!(matches!(
        s.link.add_file(ROOM, file_source("x"), " ".into(), None),
        Err(CoreError::InvalidInput { .. })
    ));

    for index in 0..10 {
        s.link
            .add_image(ROOM, photo(&format!("{index}.jpg")))
            .unwrap();
    }
    assert!(matches!(
        s.link.add_image(ROOM, photo("11.jpg")),
        Err(CoreError::InvalidInput { .. })
    ));
    assert_eq!(view(&s).max_attachments, 10);

    until("都在传", || s.fake.upload_calls() == 10).await;
    s.fake.finish(
        "0.jpg",
        Err(UplinkError::Failed(
            AttachmentFailure::Upstream,
            "坏了".into(),
        )),
    );
    until("失败", || {
        matches!(
            view(&s).attachments[0].state,
            AttachmentState::Failed { .. }
        )
    })
    .await;
    assert!(
        matches!(
            s.link.send(ROOM, "发", None),
            Err(CoreError::InvalidInput { .. })
        ),
        "附件区里有失败的不让发"
    );
    let failed = view(&s).attachments[0].id.clone();
    s.link.remove(ROOM, &failed).unwrap();
    s.link.send(ROOM, "发", None).unwrap();
    assert_eq!(view(&s).outgoing[0].attachments.len(), 9);
}

/// 系统分享进来的东西（ADR-0097）直接成一条待发消息，不带上这个房间附件区里放了一半的附件，也不动草稿。
#[tokio::test(flavor = "multi_thread")]
async fn sending_shared_items_leaves_the_tray_and_draft_alone() {
    let s = setup();
    s.link.add_image(ROOM, photo("tray.jpg")).unwrap();
    s.link
        .send_items(
            ROOM,
            " 看看这个 ",
            vec![
                super::NewItem::Image(photo("shared.jpg")),
                super::NewItem::File {
                    source: file_source("bill.pdf"),
                    name: "bill.pdf".into(),
                    mime_type: Some("application/pdf".into()),
                },
            ],
        )
        .unwrap();
    let pending = view(&s);
    assert_eq!(pending.attachments.len(), 1, "附件区里原来的那张还在");
    assert_eq!(pending.attachments[0].name, "tray.jpg");
    assert_eq!(pending.outgoing.len(), 1);
    assert_eq!(pending.outgoing[0].body, "看看这个");
    let names: Vec<_> = pending.outgoing[0]
        .attachments
        .iter()
        .map(|attachment| attachment.name.as_str())
        .collect();
    assert_eq!(names, ["shared.jpg", "bill.pdf"]);
    assert!(s.fake.drafts.lock().unwrap().is_empty(), "草稿没被当成用掉");

    until("照片在传", || {
        s.fake
            .upload_calls
            .lock()
            .unwrap()
            .iter()
            .any(|name| name == "shared.jpg")
    })
    .await;
    s.fake
        .finish("shared.jpg", Ok(uploaded("shared.jpg", false)));
    until("文件在传", || {
        s.fake
            .transfers
            .borrow()
            .active
            .iter()
            .any(|transfer| transfer.id == "t-bill.pdf")
    })
    .await;
    let file = FileId::from_bytes([9; 16]);
    s.fake.finish_file("t-bill.pdf", &file);
    until("交出去了", || s.fake.groups() == 1).await;
    let groups = s.fake.groups.lock().unwrap();
    assert_eq!(groups[0].parts.len(), 2, "照片一条、带链接和字的文字一条");
    assert!(groups[0].group_id.is_some());
}

#[tokio::test(flavor = "multi_thread")]
async fn shared_items_beyond_the_limit_are_refused() {
    let s = setup();
    let items = (0..11)
        .map(|index| super::NewItem::Image(photo(&format!("{index}.jpg"))))
        .collect();
    assert!(matches!(
        s.link.send_items(ROOM, "", items),
        Err(CoreError::InvalidInput { .. })
    ));
    assert!(view(&s).outgoing.is_empty());
}
