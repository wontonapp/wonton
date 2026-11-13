//! 一个房间的发送任务：队首的待发消息附件全部传完后，按需重传成加密的照片、把文件授权给房间，再把一组事件交给发送队列。
//! 队首没好（在传、失败了等主人）就等着，后面的也不发。

use std::sync::{Arc, Weak};

use tokio::sync::Notify;
use wonton_l10n::l10n;
use wonton_protocol::{AttachmentState, FileId, files_message_body};

use super::ComposerLink;
use super::state::{Body, Outgoing};
use super::uplink::{Handover, Part, UplinkError};
use crate::l10n::{CATALOG, language};

enum Step {
    /// 没有待发消息了，任务结束。
    Idle,
    /// 队首在等附件或者等主人。
    Wait,
    Send(Outgoing),
}

pub(super) async fn run(link: Weak<ComposerLink>, room: String, notify: Arc<Notify>) {
    loop {
        let Some(link) = link.upgrade() else {
            return;
        };
        match link.step(&room) {
            Step::Idle => return,
            Step::Wait => {
                drop(link);
                notify.notified().await;
            }
            Step::Send(outgoing) => match link.hand_over(&room, &outgoing).await {
                Ok(()) => {}
                Err(UplinkError::Offline(reason)) => {
                    tracing::info!(%reason, "composer waits for the connection before sending");
                    let online = link.uplink.online();
                    drop(link);
                    online.await;
                }
                Err(UplinkError::Failed(_, message)) => link.fail(&room, &outgoing.id, message),
            },
        }
    }
}

/// 这一组要交出去的事件：照片按顺序；只有照片时字是最后一张的说明文字；有文件时字和全部链接合成最后一条文字。
pub(super) fn parts(outgoing: &Outgoing) -> Vec<Part> {
    if let Some(reply) = &outgoing.prepared_reply {
        return vec![Part::Reply(reply.clone())];
    }
    let mut parts = Vec::new();
    let mut files: Vec<FileId> = Vec::new();
    for item in &outgoing.items {
        match &item.body {
            Body::Image {
                uploaded: Some(uploaded),
                ..
            } => parts.push(Part::Image {
                uploaded: uploaded.clone(),
                caption: None,
            }),
            Body::File {
                file: Some(file), ..
            } => files.push(file.clone()),
            _ => {}
        }
    }
    let body = outgoing.body.trim();
    if !files.is_empty() {
        parts.push(Part::Text(files_message_body(body, &files)));
    } else if let Some(Part::Image { caption, .. }) = parts.last_mut() {
        *caption = (!body.is_empty()).then(|| body.to_owned());
    } else {
        parts.push(Part::Text(body.to_owned()));
    }
    parts
}

impl ComposerLink {
    fn step(&self, room: &str) -> Step {
        let mut inner = self.lock();
        let Some(head) = inner
            .state
            .rooms
            .get_mut(room)
            .and_then(|room| room.outgoing.first_mut())
        else {
            inner.senders.remove(room);
            return Step::Idle;
        };
        if head.failure.is_some() {
            return Step::Wait;
        }
        if let Some(failed) = head
            .items
            .iter()
            .find(|item| matches!(item.state, AttachmentState::Failed { .. }))
        {
            head.failure = Some(l10n!(
                CATALOG,
                language(),
                "composer-attachment-failed",
                name = failed.name.as_str()
            ));
            if let Err(error) = self.disk.save(&inner.state) {
                tracing::warn!(%error, "could not save the composer");
            }
            Self::publish(&inner, room);
            return Step::Wait;
        }
        if head
            .items
            .iter()
            .any(|item| item.state != AttachmentState::Ready)
        {
            return Step::Wait;
        }
        Step::Send(head.clone())
    }

    fn fail(&self, room: &str, id: &str, message: String) {
        self.record(room, |state| {
            if let Some(outgoing) = state.outgoing_mut(room, id) {
                outgoing.failure = Some(message);
            }
        });
    }

    /// 明文传的照片：房间后来开了加密的话要重传。交出去一部分以后不再管：交出去的已经在发送队列里了。
    async fn reupload_if_encrypted(
        self: &Arc<Self>,
        room: &str,
        outgoing: &Outgoing,
    ) -> Result<bool, UplinkError> {
        let plain: Vec<String> = outgoing
            .items
            .iter()
            .filter(|item| {
                matches!(&item.body, Body::Image { uploaded: Some(uploaded), .. } if !uploaded.encrypted)
            })
            .map(|item| item.id.clone())
            .collect();
        if outgoing.handed > 0
            || plain.is_empty()
            || !self.uplink.room_is_encrypted(room.to_owned()).await?
        {
            return Ok(false);
        }
        self.record(room, |state| {
            for id in &plain {
                if let Some(item) = state.item_mut(room, id) {
                    if let Body::Image { uploaded, .. } = &mut item.body {
                        *uploaded = None;
                    }
                    item.state = AttachmentState::Uploading { progress: None };
                }
            }
        });
        for id in &plain {
            self.start_worker(room, id);
        }
        Ok(true)
    }

    async fn grant_files(&self, room: &str, outgoing: &Outgoing) -> Result<(), UplinkError> {
        for item in &outgoing.items {
            let Body::File {
                file: Some(file),
                granted: false,
                ..
            } = &item.body
            else {
                continue;
            };
            self.uplink.grant(file.clone(), room.to_owned()).await?;
            self.record(room, |state| {
                if let Some(Body::File { granted, .. }) =
                    state.item_mut(room, &item.id).map(|item| &mut item.body)
                {
                    *granted = true;
                }
            });
        }
        Ok(())
    }

    /// 还是队首、没被取消、没在等主人：记成正在交出，取消不了了。
    fn begin_handing(&self, room: &str, id: &str) -> bool {
        let mut inner = self.lock();
        let still_head = inner
            .state
            .rooms
            .get(room)
            .and_then(|room| room.outgoing.first())
            .is_some_and(|head| head.id == id && head.failure.is_none());
        if still_head {
            inner.handing.insert(id.to_owned());
        }
        still_head
    }

    async fn hand_over(
        self: &Arc<Self>,
        room: &str,
        outgoing: &Outgoing,
    ) -> Result<(), UplinkError> {
        if self.reupload_if_encrypted(room, outgoing).await? {
            return Ok(());
        }
        self.grant_files(room, outgoing).await?;
        if !self.begin_handing(room, &outgoing.id) {
            return Ok(());
        }
        let handed = {
            let link = Arc::downgrade(self);
            let (room, id) = (room.to_owned(), outgoing.id.clone());
            Box::new(move |count: usize| {
                if let Some(link) = link.upgrade() {
                    link.record(&room, |state| {
                        if let Some(outgoing) = state.outgoing_mut(&room, &id) {
                            outgoing.handed = count;
                        }
                    });
                }
            })
        };
        let handover = Handover {
            room: room.to_owned(),
            parts: parts(outgoing),
            group_id: (!outgoing.items.is_empty()).then(|| outgoing.group.clone()),
            reply_to: outgoing.reply_to.clone(),
            skip: outgoing.handed,
        };
        let result = self.uplink.send_group(handover, handed).await;
        self.lock().handing.remove(&outgoing.id);
        result?;
        self.sent(room, &outgoing.id);
        Ok(())
    }

    /// 整组都在发送队列里了：待发消息由 SDK 的本地回显代替。
    fn sent(&self, room: &str, id: &str) {
        let mut items = Vec::new();
        self.record(room, |state| {
            if let Some(list) = state.rooms.get_mut(room).map(|room| &mut room.outgoing)
                && let Some(position) = list.iter().position(|outgoing| outgoing.id == id)
            {
                items = list.remove(position).items;
            }
        });
        let mut inner = self.lock();
        for item in &items {
            inner.workers.remove(&item.id);
            self.disk.forget(&item.id);
        }
        drop(inner);
        self.uplink.sent(room);
    }
}
