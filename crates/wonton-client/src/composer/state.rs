//! 附件区和待发消息的状态，存盘的就是它；壳看到的 [`ComposerViewModel`] 由它算出来。

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use wonton_protocol::{
    AttachmentKind, AttachmentState, ComposerAttachment, ComposerViewModel, FileId,
    OutgoingMessage, SendState, TimelineTarget, UploadSource,
};

use super::uplink::Uploaded;

/// 附件区最多放几个附件。系统选择器一次选太多时，照片在内存里排队上传，手机吃不消。
pub(crate) const MAX_ATTACHMENTS: usize = 10;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub(crate) struct State {
    pub(crate) rooms: BTreeMap<String, Room>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub(crate) struct Room {
    pub(crate) tray: Vec<Item>,
    pub(crate) outgoing: Vec<Outgoing>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct Item {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) mime_type: Option<String>,
    pub(crate) size: Option<u64>,
    pub(crate) body: Body,
    pub(crate) state: AttachmentState,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum Body {
    /// 原图和缩略图的字节放在附件区目录里，以附件的标识命名。
    Image {
        width: Option<u32>,
        height: Option<u32>,
        thumbnail: Option<Thumbnail>,
        uploaded: Option<Uploaded>,
    },
    File {
        source: UploadSource,
        /// 文件服务里这次上传的标识，开始传了才有。
        transfer: Option<String>,
        /// 传完后的文件编号。
        file: Option<FileId>,
        /// 已经授权给房间：重试时不再授权一次。
        granted: bool,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct Thumbnail {
    pub(crate) mime_type: String,
    pub(crate) width: u32,
    pub(crate) height: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct Outgoing {
    pub(crate) id: String,
    /// 分组标记里的 `id`。
    pub(crate) group: String,
    pub(crate) body: String,
    pub(crate) reply_to: Option<String>,
    pub(crate) prepared_reply: Option<wonton_matrix::PreparedReply>,
    pub(crate) items: Vec<Item>,
    pub(crate) timestamp_ms: u64,
    /// 已经交给发送队列的事件条数：交到一半断了，下次从这里接着交，不重复发。
    pub(crate) handed: usize,
    /// 停下来等主人处理的原因（附件传不上去、授权没成、发送队列拒收）。
    pub(crate) failure: Option<String>,
}

impl Item {
    pub(crate) fn is_image(&self) -> bool {
        matches!(self.body, Body::Image { .. })
    }

    pub(crate) fn view(&self) -> ComposerAttachment {
        ComposerAttachment {
            id: self.id.clone(),
            kind: match &self.body {
                Body::Image { width, height, .. } => AttachmentKind::Image {
                    width: *width,
                    height: *height,
                },
                Body::File { .. } => AttachmentKind::File,
            },
            name: self.name.clone(),
            mime_type: self.mime_type.clone(),
            size: self.size,
            state: self.state.clone(),
        }
    }

    /// 这个附件传了多少，用来算整条消息的进度。
    fn progress(&self) -> f32 {
        match self.state {
            AttachmentState::Ready => 1.0,
            AttachmentState::Uploading {
                progress: Some(progress),
            } => progress,
            _ => 0.0,
        }
    }

    /// 在传、在等连接：重启后要接着传。
    pub(crate) fn is_pending(&self) -> bool {
        matches!(
            self.state,
            AttachmentState::Uploading { .. } | AttachmentState::Paused
        )
    }
}

impl Outgoing {
    fn view(&self) -> OutgoingMessage {
        let state = match &self.failure {
            Some(message) => SendState::Failed {
                message: message.clone(),
                recoverable: false,
            },
            None => SendState::Sending {
                progress: self.progress(),
            },
        };
        OutgoingMessage {
            id: self.id.clone(),
            body: self.body.clone(),
            attachments: self.items.iter().map(Item::view).collect(),
            timestamp_ms: self.timestamp_ms,
            state,
        }
    }

    /// 全部附件合起来的比例；只有字时不确定。
    fn progress(&self) -> Option<f32> {
        if self.items.is_empty() {
            return None;
        }
        #[expect(clippy::cast_precision_loss, reason = "at most MAX_ATTACHMENTS items")]
        let count = self.items.len() as f32;
        Some(self.items.iter().map(Item::progress).sum::<f32>() / count)
    }
}

impl State {
    pub(crate) fn view(&self, room_id: &str) -> ComposerViewModel {
        let room = self.rooms.get(room_id);
        ComposerViewModel {
            target: TimelineTarget {
                room_id: room_id.to_owned(),
                thread_root: None,
            },
            attachments: room
                .map(|room| room.tray.iter().map(Item::view).collect())
                .unwrap_or_default(),
            outgoing: room
                .map(|room| room.outgoing.iter().map(Outgoing::view).collect())
                .unwrap_or_default(),
            max_attachments: u32::try_from(MAX_ATTACHMENTS).expect("a small constant"),
        }
    }

    /// 附件区或待发消息里的一个附件。
    pub(crate) fn item_mut(&mut self, room: &str, id: &str) -> Option<&mut Item> {
        let room = self.rooms.get_mut(room)?;
        room.tray
            .iter_mut()
            .chain(
                room.outgoing
                    .iter_mut()
                    .flat_map(|outgoing| &mut outgoing.items),
            )
            .find(|item| item.id == id)
    }

    pub(crate) fn item(&self, room: &str, id: &str) -> Option<&Item> {
        let room = self.rooms.get(room)?;
        room.tray
            .iter()
            .chain(room.outgoing.iter().flat_map(|outgoing| &outgoing.items))
            .find(|item| item.id == id)
    }

    pub(crate) fn outgoing_mut(&mut self, room: &str, id: &str) -> Option<&mut Outgoing> {
        self.rooms
            .get_mut(room)?
            .outgoing
            .iter_mut()
            .find(|outgoing| outgoing.id == id)
    }

    /// 空了的房间不留在存盘的内容里。
    pub(crate) fn prune(&mut self, room: &str) {
        if self
            .rooms
            .get(room)
            .is_some_and(|room| room.tray.is_empty() && room.outgoing.is_empty())
        {
            self.rooms.remove(room);
        }
    }
}
