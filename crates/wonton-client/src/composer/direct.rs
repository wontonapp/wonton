//! 不经附件区、直接变成一条待发消息的发送：系统分享进来的东西（ADR-0097）。上传、授权、一组事件和重试都和附件区一样，
//! 只是不碰这个房间的附件区和草稿：主人在这个房间里放了一半的附件不会被一起发出去。

use std::sync::Arc;

use wonton_l10n::l10n;
use wonton_protocol::{CoreError, ImageUpload, UploadSource};

use super::state::{MAX_ATTACHMENTS, Outgoing};
use super::{
    ComposerLink, check_file_name, check_image_type, file_item, invalid, now_ms, random_hex,
};
use crate::l10n::{CATALOG, language};

/// 一次发送里的一项。
pub(crate) enum NewItem {
    Image(ImageUpload),
    File {
        source: UploadSource,
        name: String,
        mime_type: Option<String>,
    },
}

impl ComposerLink {
    /// 把这些附件和字一起变成这个房间的一条待发消息，马上开始上传。
    pub(crate) fn send_items(
        self: &Arc<Self>,
        room: &str,
        body: &str,
        items: Vec<NewItem>,
    ) -> Result<(), CoreError> {
        if body.trim().is_empty() && items.is_empty() {
            return Err(invalid(l10n!(CATALOG, language(), "chat-empty-message")));
        }
        if items.len() > MAX_ATTACHMENTS {
            return Err(invalid(l10n!(
                CATALOG,
                language(),
                "composer-tray-full",
                max = MAX_ATTACHMENTS
            )));
        }
        for item in &items {
            match item {
                NewItem::Image(image) => check_image_type(image)?,
                NewItem::File { name, .. } => check_file_name(name)?,
            }
        }
        self.uplink.check_send(room, None)?;
        let mut built = Vec::with_capacity(items.len());
        for item in items {
            let item = match item {
                NewItem::Image(image) => self.image_item(image),
                NewItem::File {
                    source,
                    name,
                    mime_type,
                } => Ok(file_item(source, name, mime_type)),
            };
            match item {
                Ok(item) => built.push(item),
                Err(error) => {
                    for item in &built {
                        self.disk.forget(&item.id);
                    }
                    return Err(error);
                }
            }
        }
        let ids: Vec<String> = built.iter().map(|item| item.id.clone()).collect();
        let group = random_hex();
        let outgoing_id = format!("outgoing-{group}");
        let ((), saved) = self.change(room, |state| {
            state
                .rooms
                .entry(room.to_owned())
                .or_default()
                .outgoing
                .push(Outgoing {
                    id: outgoing_id.clone(),
                    group,
                    body: body.trim().to_owned(),
                    reply_to: None,
                    prepared_reply: None,
                    items: built,
                    timestamp_ms: now_ms(),
                    handed: 0,
                    failure: None,
                });
        });
        if let Err(error) = saved {
            // 存不上就不发：主人看到错误，分享还在。撤回的这次存盘多半也存不上，不再报。
            let _ = self.change(room, |state| {
                if let Some(room) = state.rooms.get_mut(room) {
                    room.outgoing.retain(|outgoing| outgoing.id != outgoing_id);
                }
            });
            for id in &ids {
                self.disk.forget(id);
            }
            return Err(error);
        }
        for id in &ids {
            self.start_worker(room, id);
        }
        self.kick(room);
        Ok(())
    }
}
