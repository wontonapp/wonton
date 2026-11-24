//! 输入框的附件区和一起发送（ADR-0094）。
//!
//! 附件放进附件区就开始上传。点发送后，字、回复对象和附件变成一条待发消息；这一条的附件全部传完、文件授权给房间后，
//! 由这个房间的发送任务（[`sender`]）把一组事件按顺序交给 matrix-sdk 的发送队列。同一房间的待发消息按点发送的顺序交出。

mod direct;
mod sender;
mod state;
mod store;
pub(crate) mod uplink;
mod worker;

pub(crate) use direct::NewItem;

#[cfg(test)]
mod tests;

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard};

use tokio::runtime::Handle;
use tokio::sync::{Notify, watch};
use tokio::task::JoinHandle;
use wonton_l10n::l10n;
use wonton_protocol::{
    AttachmentFailure, AttachmentState, ComposerMention, ComposerViewModel, CoreError, ImageUpload,
    RoomTransfers, UploadSource,
};

use crate::l10n::{CATALOG, language};
use state::{Body, Item, MAX_ATTACHMENTS, Outgoing, State, Thumbnail};
use store::Disk;
use uplink::Uplink;

pub(crate) struct ComposerLink {
    runtime: Handle,
    uplink: Arc<dyn Uplink>,
    disk: Disk,
    inner: Mutex<Inner>,
}

#[derive(Default)]
struct Inner {
    state: State,
    /// 壳订阅过的房间。
    views: HashMap<String, watch::Sender<ComposerViewModel>>,
    /// 每个附件的上传任务，按附件的标识。
    workers: HashMap<String, JoinHandle<()>>,
    /// 在跑的发送任务，按房间：叫醒它看看队首能不能发了。
    senders: HashMap<String, Arc<Notify>>,
    /// 正在交给发送队列的待发消息：这时候不能取消，交出去的那几条已经在 SDK 手里。
    handing: HashSet<String>,
    /// @ 列表的候选，按房间；由聊天那边跟着成员和 Bot 名单补。
    mentions: HashMap<String, Vec<ComposerMention>>,
    /// 假数据入口（`ui-fake`）下，下次放进附件区的东西要标成上传失败的房间。
    fail_next: HashSet<String>,
    /// 假数据入口（`ui-fake`）下，界面测试叫停的那间：下一张照片进附件区以后在这里等，等的一头在下面。
    holds: HashMap<String, std::sync::mpsc::Receiver<()>>,
    /// 上面那些叫停的放手一头，[`ComposerLink::release_held_for_test`] 用它把等着的放走。
    hold_releases: HashMap<String, std::sync::mpsc::Sender<()>>,
}

impl Inner {
    fn view(&self, room: &str) -> ComposerViewModel {
        self.state
            .view(room, self.mentions.get(room).map_or(&[], Vec::as_slice))
    }
}

fn invalid(reason: String) -> CoreError {
    CoreError::InvalidInput { reason }
}

fn random_hex() -> String {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).expect("the OS random source is unavailable");
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| {
            u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX)
        })
}

fn attachment_not_found() -> CoreError {
    invalid(l10n!(CATALOG, language(), "composer-attachment-not-found"))
}

fn outgoing_not_found() -> CoreError {
    invalid(l10n!(CATALOG, language(), "composer-outgoing-not-found"))
}

fn check_image_type(image: &ImageUpload) -> Result<(), CoreError> {
    if image.mime_type.starts_with("image/") {
        return Ok(());
    }
    Err(invalid(l10n!(
        CATALOG,
        language(),
        "chat-not-image-type",
        file_type = image.mime_type.as_str()
    )))
}

fn check_file_name(name: &str) -> Result<(), CoreError> {
    if name.trim().is_empty() {
        return Err(invalid(l10n!(CATALOG, language(), "files-name-required")));
    }
    Ok(())
}

fn file_item(source: UploadSource, name: String, mime_type: Option<String>) -> Item {
    Item {
        id: random_hex(),
        name,
        mime_type,
        size: None,
        body: Body::File {
            source,
            transfer: None,
            file: None,
            granted: false,
        },
        state: AttachmentState::Uploading { progress: None },
    }
}

impl ComposerLink {
    /// 读回上次的附件区和待发消息：没传完的接着传，待发的接着发。
    pub(crate) fn new(runtime: Handle, uplink: Arc<dyn Uplink>, dir: &Path) -> Arc<Self> {
        let disk = Disk::new(dir.join("composer"));
        let state = disk.load();
        let link = Arc::new(Self {
            runtime,
            uplink,
            disk,
            inner: Mutex::new(Inner {
                state,
                ..Inner::default()
            }),
        });
        let (pending, rooms) = {
            let inner = link.lock();
            let pending: Vec<(String, String)> = inner
                .state
                .rooms
                .iter()
                .flat_map(|(room_id, room)| {
                    room.tray
                        .iter()
                        .chain(room.outgoing.iter().flat_map(|outgoing| &outgoing.items))
                        .filter(|item| item.is_pending())
                        .map(|item| (room_id.clone(), item.id.clone()))
                })
                .collect();
            let rooms: Vec<String> = inner.state.rooms.keys().cloned().collect();
            (pending, rooms)
        };
        for (room, id) in pending {
            link.start_worker(&room, &id);
        }
        for room in rooms {
            link.kick(&room);
        }
        link
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub(crate) fn subscribe(&self, room: &str) -> watch::Receiver<ComposerViewModel> {
        let mut inner = self.lock();
        let view = inner.view(room);
        inner
            .views
            .entry(room.to_owned())
            .or_insert_with(|| watch::Sender::new(view))
            .subscribe()
    }

    /// 从房间的传输条里拿掉附件区自己的上传：它们画在附件区和待发消息里，传输条再列一遍就重了；又因为不带房间，
    /// 不拿掉的话每个房间都会列。
    pub(crate) fn hide_own_uploads(&self, mut listed: RoomTransfers) -> RoomTransfers {
        let own: HashSet<String> = {
            let inner = self.lock();
            inner
                .state
                .rooms
                .values()
                .flat_map(|room| {
                    room.tray
                        .iter()
                        .chain(room.outgoing.iter().flat_map(|outgoing| &outgoing.items))
                })
                .filter_map(|item| match &item.body {
                    Body::File {
                        transfer: Some(transfer),
                        ..
                    } => Some(transfer.clone()),
                    _ => None,
                })
                .collect()
        };
        listed.here.retain(|transfer| !own.contains(&transfer.id));
        listed
    }

    fn publish(inner: &Inner, room: &str) {
        if let Some(sender) = inner.views.get(room) {
            let view = inner.view(room);
            sender.send_if_modified(|current| {
                let changed = *current != view;
                *current = view;
                changed
            });
        }
    }

    /// 换掉一个房间的 @ 候选；和现有的不一样才推给壳。不存盘，重启后由聊天那边重新算。
    pub(crate) fn set_mentions(&self, room: &str, candidates: Vec<ComposerMention>) {
        let mut inner = self.lock();
        if inner.mentions.get(room) == Some(&candidates) {
            return;
        }
        if candidates.is_empty() {
            inner.mentions.remove(room);
        } else {
            inner.mentions.insert(room.to_owned(), candidates);
        }
        Self::publish(&inner, room);
    }

    /// 改状态、存盘、推给壳。主人的操作要知道存没存上，返回存盘的结果。
    fn change<R>(&self, room: &str, f: impl FnOnce(&mut State) -> R) -> (R, Result<(), CoreError>) {
        let mut inner = self.lock();
        let result = f(&mut inner.state);
        inner.state.prune(room);
        let saved = self.disk.save(&inner.state);
        Self::publish(&inner, room);
        (result, saved)
    }

    /// 后台任务的状态变化：存不上只记日志，内存里的状态照常往下走。
    fn record(&self, room: &str, f: impl FnOnce(&mut State)) {
        if let ((), Err(error)) = self.change(room, f) {
            tracing::warn!(%error, "could not save the composer");
        }
    }

    /// 只推给壳不存盘：上传进度变得很频繁，重启后反正从头算。失败是等主人处理的稳定态，进度和等联网不能把它盖掉
    /// （重试走 [`restart`](Self::restart)，先把状态改成上传中再起 worker）。
    fn show(&self, room: &str, id: &str, state: AttachmentState) {
        let mut inner = self.lock();
        let Some(item) = inner.state.item_mut(room, id) else {
            return;
        };
        if matches!(item.state, AttachmentState::Failed { .. }) {
            return;
        }
        item.state = state;
        Self::publish(&inner, room);
    }

    fn check_room_has_space(&self, room: &str) -> Result<(), CoreError> {
        let inner = self.lock();
        let used = inner
            .state
            .rooms
            .get(room)
            .map_or(0, |room| room.tray.len());
        if used >= MAX_ATTACHMENTS {
            return Err(invalid(l10n!(
                CATALOG,
                language(),
                "composer-tray-full",
                max = MAX_ATTACHMENTS
            )));
        }
        Ok(())
    }

    fn add(self: &Arc<Self>, room: &str, item: Item) -> Result<String, CoreError> {
        let id = item.id.clone();
        let ((), saved) = self.change(room, |state| {
            state
                .rooms
                .entry(room.to_owned())
                .or_default()
                .tray
                .push(item);
        });
        if let Err(error) = saved {
            // 存不上就不放进附件区：主人看到错误，附件区里也没有它。撤回的这次存盘多半也存不上，不再报。
            let _ = self.change(room, |state| {
                if let Some(room) = state.rooms.get_mut(room) {
                    room.tray.retain(|item| item.id != id);
                }
            });
            self.disk.forget(&id);
            return Err(error);
        }
        if self.lock().fail_next.remove(room) {
            self.show(
                room,
                &id,
                AttachmentState::Failed {
                    reason: AttachmentFailure::Upstream,
                    message: "ui-fake: upload failed on purpose".into(),
                },
            );
        } else {
            self.start_worker(room, &id);
        }
        Ok(id)
    }

    pub(crate) fn add_image(
        self: &Arc<Self>,
        room: &str,
        image: ImageUpload,
    ) -> Result<String, CoreError> {
        check_image_type(&image)?;
        self.uplink.check_send(room, None)?;
        self.check_room_has_space(room)?;
        let item = self.image_item(image)?;
        self.add(room, item)
    }

    /// 假数据入口（`ui-fake`）没有登录，照样放进附件区：只跳过「房间能发」的检查，上传由 worker 照常等联网。
    #[cfg(feature = "ui-fake")]
    pub(crate) fn add_image_offline(
        self: &Arc<Self>,
        room: &str,
        image: ImageUpload,
    ) -> Result<String, CoreError> {
        check_image_type(&image)?;
        self.check_room_has_space(room)?;
        let item = self.image_item(image)?;
        let id = self.add(room, item)?;
        // 界面测试叫停时停在照片已经进附件区、还没发这一步；先放开锁，放行的调用还要进来。
        let wait = self.lock().holds.remove(room);
        if let Some(wait) = wait {
            let _ = wait.recv();
        }
        Ok(id)
    }

    /// 同 [`add_image_offline`](Self::add_image_offline)，文件那侧。
    #[cfg(feature = "ui-fake")]
    pub(crate) fn add_file_offline(
        self: &Arc<Self>,
        room: &str,
        source: UploadSource,
        name: String,
        mime_type: Option<String>,
    ) -> Result<String, CoreError> {
        check_file_name(&name)?;
        self.check_room_has_space(room)?;
        let item = file_item(source, name, mime_type);
        self.add(room, item)
    }

    pub(crate) fn add_file(
        self: &Arc<Self>,
        room: &str,
        source: UploadSource,
        name: String,
        mime_type: Option<String>,
    ) -> Result<String, CoreError> {
        check_file_name(&name)?;
        self.uplink.check_send(room, None)?;
        self.check_room_has_space(room)?;
        self.add(room, file_item(source, name, mime_type))
    }

    /// 照片的字节存进附件区目录，返回还没放进任何地方的一项。
    fn image_item(&self, image: ImageUpload) -> Result<Item, CoreError> {
        let id = random_hex();
        self.disk.save_image(
            &id,
            &image.data,
            image
                .thumbnail
                .as_ref()
                .map(|thumbnail| thumbnail.data.as_slice()),
        )?;
        Ok(Item {
            id,
            name: image.filename,
            mime_type: Some(image.mime_type),
            size: Some(u64::try_from(image.data.len()).unwrap_or(u64::MAX)),
            body: Body::Image {
                width: image.width,
                height: image.height,
                thumbnail: image.thumbnail.map(|thumbnail| Thumbnail {
                    mime_type: thumbnail.mime_type,
                    width: thumbnail.width,
                    height: thumbnail.height,
                }),
                uploaded: None,
            },
            state: AttachmentState::Uploading { progress: None },
        })
    }

    /// 停下附件的上传、删掉本机存的字节。
    fn discard(&self, inner: &mut Inner, item: &Item) {
        if let Some(worker) = inner.workers.remove(&item.id) {
            worker.abort();
        }
        if let Body::File {
            transfer: Some(transfer),
            ..
        } = &item.body
        {
            self.uplink.drop_file(transfer);
        }
        self.disk.forget(&item.id);
    }

    pub(crate) fn remove(&self, room: &str, id: &str) -> Result<(), CoreError> {
        let mut inner = self.lock();
        let tray = inner
            .state
            .rooms
            .get_mut(room)
            .map(|room| &mut room.tray)
            .ok_or_else(attachment_not_found)?;
        let position = tray
            .iter()
            .position(|item| item.id == id)
            .ok_or_else(attachment_not_found)?;
        let item = tray.remove(position);
        self.discard(&mut inner, &item);
        inner.state.prune(room);
        let saved = self.disk.save(&inner.state);
        Self::publish(&inner, room);
        saved
    }

    /// 失败的附件从头再传；重置附件和所属待发消息必须一起完成，发送任务不能再看到旧的失败。
    fn restart(self: &Arc<Self>, room: &str, id: &str) {
        let mut dropped = None;
        let mut restarted = false;
        self.record(room, |state| {
            let Some(item) = state.item_mut(room, id) else {
                return;
            };
            if !matches!(item.state, AttachmentState::Failed { .. }) {
                return;
            }
            if let Body::File { transfer, .. } = &mut item.body {
                dropped = transfer.take();
            }
            item.state = AttachmentState::Uploading { progress: None };
            restarted = true;
            if let Some(outgoing) = state.rooms.get_mut(room).and_then(|room| {
                room.outgoing
                    .iter_mut()
                    .find(|outgoing| outgoing.items.iter().any(|item| item.id == id))
            }) {
                outgoing.failure = None;
            }
        });
        if !restarted {
            return;
        }
        if let Some(transfer) = dropped {
            self.uplink.drop_file(&transfer);
        }
        self.start_worker(room, id);
    }

    pub(crate) fn retry(self: &Arc<Self>, room: &str, id: &str) -> Result<(), CoreError> {
        let failed = {
            let inner = self.lock();
            let item = inner
                .state
                .item(room, id)
                .ok_or_else(attachment_not_found)?;
            matches!(item.state, AttachmentState::Failed { .. })
        };
        if failed {
            self.restart(room, id);
        }
        Ok(())
    }

    pub(crate) fn thumbnail(&self, room: &str, id: &str) -> Result<Vec<u8>, CoreError> {
        let is_image = self.lock().state.item(room, id).is_some_and(Item::is_image);
        if !is_image {
            return Err(attachment_not_found());
        }
        match self.disk.thumbnail(id)? {
            Some(thumbnail) => Ok(thumbnail),
            None => self.disk.image(id),
        }
    }

    /// 附件区里有一张没传上去的照片或文件时，点发送会被拒：重试和删都在附件区里，这里只给原因。
    pub(crate) fn reject_if_failed(&self, room: &str) -> Result<(), CoreError> {
        if self.has_failed(room) {
            return Err(invalid(l10n!(CATALOG, language(), "composer-has-failed")));
        }
        Ok(())
    }

    fn has_failed(&self, room: &str) -> bool {
        let inner = self.lock();
        inner.state.rooms.get(room).is_some_and(|room| {
            room.tray
                .iter()
                .any(|item| matches!(item.state, AttachmentState::Failed { .. }))
        })
    }

    /// 假数据入口（`ui-fake`）：下一条放进附件区的东西直接标成上传失败。
    /// 相册面板选完就发，没有机会在中间把它标红。
    pub(crate) fn fail_next_for_test(&self, room: &str) {
        self.lock().fail_next.insert(room.to_owned());
    }

    /// 假数据入口（`ui-fake`）：下一张放进附件区的照片在进附件区以后停住，直到 [`Self::release_held_for_test`]。
    /// 转码和发送之间只有很短一段，界面测试靠它停在那里看点两下、等的时候又打字的样子。
    pub(crate) fn hold_next_for_test(&self, room: &str) {
        let (release, wait) = std::sync::mpsc::channel();
        let mut inner = self.lock();
        inner.holds.insert(room.to_owned(), wait);
        inner.hold_releases.insert(room.to_owned(), release);
    }

    /// 放走 [`Self::hold_next_for_test`] 停住的那张照片。
    pub(crate) fn release_held_for_test(&self, room: &str) {
        if let Some(release) = self.lock().hold_releases.remove(room) {
            let _ = release.send(());
        }
    }

    /// 假数据入口（`ui-fake`）没有真上传，界面测试靠它把附件标成上传失败，走到被拒的分支。
    pub(crate) fn fail_for_test(&self, room: &str, id: &str) -> Result<(), CoreError> {
        let found = {
            let inner = self.lock();
            inner.state.item(room, id).is_some()
        };
        if !found {
            return Err(attachment_not_found());
        }
        self.record(room, |state| {
            if let Some(item) = state.item_mut(room, id) {
                item.state = AttachmentState::Failed {
                    reason: AttachmentFailure::Upstream,
                    message: "ui-fake: upload failed on purpose".into(),
                };
            }
        });
        Ok(())
    }

    /// 假数据入口（`ui-fake`）没有真发送：发出去以后把附件区清掉，和真发送一样；存不上只记日志。
    pub(crate) fn clear_for_test(&self, room: &str) {
        self.record(room, |state| {
            if let Some(room) = state.rooms.get_mut(room) {
                room.tray.clear();
            }
        });
    }

    pub(crate) fn send(
        self: &Arc<Self>,
        room: &str,
        body: &str,
        reply_to: Option<String>,
        mentions: &[String],
    ) -> Result<(), CoreError> {
        {
            let inner = self.lock();
            let tray = inner.state.rooms.get(room).map(|room| room.tray.as_slice());
            if body.trim().is_empty() && tray.unwrap_or_default().is_empty() {
                return Err(invalid(l10n!(CATALOG, language(), "chat-empty-message")));
            }
        }
        self.reject_if_failed(room)?;
        self.uplink.check_send(room, reply_to.as_deref())?;
        let ((), saved) = self.change(room, |state| {
            let room = state.rooms.entry(room.to_owned()).or_default();
            let group = random_hex();
            room.outgoing.push(Outgoing {
                id: format!("outgoing-{group}"),
                group,
                body: body.trim().to_owned(),
                mentions: mentions.to_vec(),
                reply_to,
                prepared_reply: None,
                items: std::mem::take(&mut room.tray),
                timestamp_ms: now_ms(),
                handed: 0,
                failure: None,
            });
        });
        saved?;
        self.uplink.forget_draft(room, body);
        self.kick(room);
        Ok(())
    }

    /// 通知只能发文字，不能消耗同一房间正在编辑的草稿或附件区。
    pub(crate) fn send_push_reply(
        self: &Arc<Self>,
        room: &str,
        body: &str,
        reply: wonton_matrix::PreparedReply,
    ) -> Result<(), CoreError> {
        self.uplink.check_send(room, None)?;
        let ((), saved) = self.change(room, |state| {
            let group = random_hex();
            state
                .rooms
                .entry(room.to_owned())
                .or_default()
                .outgoing
                .push(Outgoing {
                    id: format!("outgoing-{group}"),
                    group,
                    body: body.trim().to_owned(),
                    mentions: Vec::new(),
                    reply_to: None,
                    prepared_reply: Some(reply),
                    items: Vec::new(),
                    timestamp_ms: now_ms(),
                    handed: 0,
                    failure: None,
                });
        });
        saved?;
        self.kick(room);
        Ok(())
    }

    pub(crate) async fn wait_for_handover(&self, room: &str) {
        let mut view = self.subscribe(room);
        let _ = view
            .wait_for(|view| {
                view.outgoing.is_empty()
                    || view.outgoing.iter().any(|outgoing| {
                        matches!(outgoing.state, wonton_protocol::SendState::Failed { .. })
                    })
            })
            .await;
    }

    pub(crate) fn retry_outgoing(self: &Arc<Self>, room: &str, id: &str) -> Result<(), CoreError> {
        let failed_items = {
            let inner = self.lock();
            let outgoing = inner
                .state
                .rooms
                .get(room)
                .and_then(|room| room.outgoing.iter().find(|outgoing| outgoing.id == id))
                .ok_or_else(outgoing_not_found)?;
            if outgoing.failure.is_none() {
                return Ok(());
            }
            outgoing
                .items
                .iter()
                .filter(|item| matches!(item.state, AttachmentState::Failed { .. }))
                .map(|item| item.id.clone())
                .collect::<Vec<_>>()
        };
        for item in failed_items {
            self.restart(room, &item);
        }
        self.record(room, |state| {
            if let Some(outgoing) = state.outgoing_mut(room, id) {
                outgoing.failure = None;
            }
        });
        self.kick(room);
        Ok(())
    }

    pub(crate) fn cancel_outgoing(self: &Arc<Self>, room: &str, id: &str) -> Result<(), CoreError> {
        let saved = {
            let mut inner = self.lock();
            if inner.handing.contains(id) {
                return Err(outgoing_not_found());
            }
            let list = inner
                .state
                .rooms
                .get_mut(room)
                .map(|room| &mut room.outgoing)
                .ok_or_else(outgoing_not_found)?;
            let position = list
                .iter()
                .position(|outgoing| outgoing.id == id)
                .ok_or_else(outgoing_not_found)?;
            let outgoing = list.remove(position);
            for item in &outgoing.items {
                self.discard(&mut inner, item);
            }
            inner.state.prune(room);
            let saved = self.disk.save(&inner.state);
            Self::publish(&inner, room);
            saved
        };
        self.kick(room);
        saved
    }

    fn start_worker(self: &Arc<Self>, room: &str, id: &str) {
        let task = self.runtime.spawn(worker::upload(
            Arc::downgrade(self),
            room.to_owned(),
            id.to_owned(),
        ));
        if let Some(previous) = self.lock().workers.insert(id.to_owned(), task) {
            previous.abort();
        }
    }

    /// 叫醒这个房间的发送任务，没在跑就开一个。
    fn kick(self: &Arc<Self>, room: &str) {
        let mut inner = self.lock();
        if let Some(notify) = inner.senders.get(room) {
            notify.notify_one();
            return;
        }
        let has_outgoing = inner
            .state
            .rooms
            .get(room)
            .is_some_and(|room| !room.outgoing.is_empty());
        if !has_outgoing {
            return;
        }
        let notify = Arc::new(Notify::new());
        inner.senders.insert(room.to_owned(), notify.clone());
        self.runtime
            .spawn(sender::run(Arc::downgrade(self), room.to_owned(), notify));
    }
}
