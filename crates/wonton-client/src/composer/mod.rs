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
    AttachmentState, ComposerViewModel, CoreError, ImageUpload, RoomTransfers, UploadSource,
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
        let view = inner.state.view(room);
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
            let view = inner.state.view(room);
            sender.send_if_modified(|current| {
                let changed = *current != view;
                *current = view;
                changed
            });
        }
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

    /// 只推给壳不存盘：上传进度变得很频繁，重启后反正从头算。
    fn show(&self, room: &str, id: &str, state: AttachmentState) {
        let mut inner = self.lock();
        let Some(item) = inner.state.item_mut(room, id) else {
            return;
        };
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
        self.start_worker(room, &id);
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

    /// 失败的附件从头再传；文件丢掉上次的传输重新开始。
    fn restart(self: &Arc<Self>, room: &str, id: &str) {
        let mut dropped = None;
        self.record(room, |state| {
            let Some(item) = state.item_mut(room, id) else {
                return;
            };
            if let Body::File { transfer, .. } = &mut item.body {
                dropped = transfer.take();
            }
            item.state = AttachmentState::Uploading { progress: None };
        });
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

    pub(crate) fn send(
        self: &Arc<Self>,
        room: &str,
        body: &str,
        reply_to: Option<String>,
    ) -> Result<(), CoreError> {
        let has_failed = {
            let inner = self.lock();
            let tray = inner.state.rooms.get(room).map(|room| room.tray.as_slice());
            let tray = tray.unwrap_or_default();
            if body.trim().is_empty() && tray.is_empty() {
                return Err(invalid(l10n!(CATALOG, language(), "chat-empty-message")));
            }
            tray.iter()
                .any(|item| matches!(item.state, AttachmentState::Failed { .. }))
        };
        if has_failed {
            return Err(invalid(l10n!(CATALOG, language(), "composer-has-failed")));
        }
        self.uplink.check_send(room, reply_to.as_deref())?;
        let ((), saved) = self.change(room, |state| {
            let room = state.rooms.entry(room.to_owned()).or_default();
            let group = random_hex();
            room.outgoing.push(Outgoing {
                id: format!("outgoing-{group}"),
                group,
                body: body.trim().to_owned(),
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
        self.record(room, |state| {
            if let Some(outgoing) = state.outgoing_mut(room, id) {
                outgoing.failure = None;
            }
        });
        for item in failed_items {
            self.restart(room, &item);
        }
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
