//! 附件区的行为测试：给定一串操作，断言壳看到的 ViewModel 和交给 Matrix 的事件。Matrix 和文件服务换成测试替身。

mod failure_tests;
mod grouping_tests;
mod input_tests;
mod persistence_tests;
mod queue_tests;
mod resume_tests;
mod retry_tests;
mod share_tests;
mod transfer_strip_tests;
mod upload_lifecycle_tests;

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

use super::uplink::{Handed, Handover, Part, Progress, Uplink, UplinkError, Uploaded};
use super::{ComposerLink, NewItem, store};

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
    file_calls: Mutex<Vec<String>>,
    encrypted: Mutex<bool>,
    online: watch::Sender<bool>,
    transfers: watch::Sender<TransfersViewModel>,
    /// 授权和交出按发生的顺序记一份，看授权是不是在交出之前。
    log: Mutex<Vec<String>>,
    groups: Mutex<Vec<Group>>,
    /// 交出这么多条后断线一次。
    cut_after: Mutex<Option<usize>>,
    drafts: Mutex<Vec<String>>,
    send_error: Mutex<Option<UplinkError>>,
    /// 交出时卡在这里，等测试放行：用来停在“已交给服务器、消息还没落地”的中间状态。
    send_gate: Mutex<Option<oneshot::Receiver<()>>>,
}

impl Fake {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            uploads: Mutex::default(),
            upload_calls: Mutex::default(),
            file_calls: Mutex::default(),
            encrypted: Mutex::new(false),
            online: watch::Sender::new(true),
            transfers: watch::Sender::new(TransfersViewModel::default()),
            log: Mutex::default(),
            groups: Mutex::default(),
            cut_after: Mutex::default(),
            drafts: Mutex::default(),
            send_error: Mutex::default(),
            send_gate: Mutex::default(),
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
        let id = {
            let mut calls = self.file_calls.lock().unwrap();
            calls.push(name.clone());
            let attempt = calls.iter().filter(|seen| *seen == &name).count();
            if attempt == 1 {
                format!("t-{name}")
            } else {
                format!("t-{name}-{attempt}")
            }
        };
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
        self.transfers
            .send_modify(|view| view.active.retain(|seen| seen.id != transfer));
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
        if let Some(error) = self.send_error.lock().unwrap().take() {
            return Box::pin(async move { Err(error) });
        }
        let gate = self.send_gate.lock().unwrap().take();
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
            if let Some(gate) = gate {
                let _ = gate.await;
            }
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

fn attachment(setup: &Setup, id: &str) -> AttachmentState {
    view(setup)
        .attachments
        .iter()
        .find(|item| item.id == id)
        .expect("附件还在")
        .state
        .clone()
}
