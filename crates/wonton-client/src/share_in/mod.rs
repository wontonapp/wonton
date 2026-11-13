//! 从系统分享面板分享进云吞（ADR-0097）。壳把收到的东西放进收件目录，这里认领、给出分享页，主人点发送时交给附件区
//! （[`ComposerLink::send_items`](crate::composer::ComposerLink)）发成一条待发消息。
//!
//! 分享页不跟服务器走：没配对时也能认领，配好以后接着发；换服务器时去向表跟着换。所以它挂在 `ClientCore` 上，
//! 不在作用域里。

mod inbox;
#[cfg(test)]
mod tests;
mod view;

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::SystemTime;

use serde::{Deserialize, Serialize};
use tokio::sync::watch;
use wonton_l10n::l10n;
use wonton_protocol::{
    AttachmentKind, ChatViewModel, CoreError, PendingShare, ShareInImage, ShareInItem,
    ShareInViewModel, UploadSource,
};

use crate::composer::NewItem;
use crate::l10n::{CATALOG, language};
pub(crate) use inbox::Claimed;

/// 记住上次发去的房间：下次分享默认选它。
const LAST_TARGET_FILE: &str = "share-in.json";

#[derive(Default, Serialize, Deserialize)]
struct Remembered {
    last_room: Option<String>,
}

pub(crate) struct ShareInLink {
    root: PathBuf,
    pending: watch::Sender<Option<Claimed>>,
    last_room: Mutex<Option<String>>,
}

/// 要发出去的一次分享：附件按分享进来的顺序。
pub(crate) struct Outbound {
    pub(crate) token: String,
    pub(crate) items: Vec<NewItem>,
    /// 有文件：发出去以后目录要留着给 tus 续传。
    pub(crate) has_files: bool,
}

fn invalid(reason: String) -> CoreError {
    CoreError::InvalidInput { reason }
}

fn nothing_pending() -> CoreError {
    invalid(l10n!(CATALOG, language(), "share-in-nothing-pending"))
}

impl ShareInLink {
    /// 启动时清掉放了超过一天的分享。
    pub(crate) fn new(root: &Path) -> Self {
        inbox::clean(root, SystemTime::now());
        let remembered: Remembered = fs::read(root.join(LAST_TARGET_FILE))
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        Self {
            root: root.to_owned(),
            pending: watch::Sender::new(None),
            last_room: Mutex::new(remembered.last_room),
        }
    }

    pub(crate) fn subscribe(&self) -> watch::Receiver<Option<Claimed>> {
        self.pending.subscribe()
    }

    fn last_room(&self) -> Option<String> {
        self.last_room
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    /// 认领收件目录里的一次分享，顶掉还没发的上一次（上一次的文件删掉）。
    pub(crate) fn claim(&self, token: &str) -> Result<(), CoreError> {
        inbox::clean(&self.root, SystemTime::now());
        let claimed = inbox::claim(&self.root, token)?;
        if let Some(previous) = self.pending.send_replace(Some(claimed))
            && previous.token != token
        {
            inbox::remove(&previous.dir);
        }
        Ok(())
    }

    /// 删掉一项；`None` 是整个不发了。条目删光、也没有文字时等于不发了。
    pub(crate) fn discard(&self, item_id: Option<&str>) -> Result<(), CoreError> {
        let mut outcome = Ok(());
        let mut drop_dir = None;
        self.pending.send_if_modified(|pending| {
            let Some(share) = pending.as_mut() else {
                outcome = Err(nothing_pending());
                return false;
            };
            let Some(item_id) = item_id else {
                drop_dir = pending.take().map(|share| share.dir);
                return true;
            };
            let Some(position) = share.items.iter().position(|item| item.id == item_id) else {
                outcome = Err(invalid(l10n!(
                    CATALOG,
                    language(),
                    "share-in-item-not-found"
                )));
                return false;
            };
            let item = share.items.remove(position);
            inbox::remove(&item.path);
            if share.items.is_empty() && share.text.is_empty() {
                drop_dir = pending.take().map(|share| share.dir);
            }
            true
        });
        if let Some(dir) = drop_dir {
            inbox::remove(&dir);
        }
        outcome
    }

    /// 按壳交回的照片把这次分享变成附件，不改分享本身：发送没成时主人还能换个去向再发。
    pub(crate) fn outbound(&self, images: Vec<ShareInImage>) -> Result<Outbound, CoreError> {
        let share = self.pending.borrow().clone().ok_or_else(nothing_pending)?;
        let mut images = images;
        let mut items = Vec::with_capacity(share.items.len());
        let mut has_files = false;
        for item in share.items {
            match item.kind {
                AttachmentKind::Image { .. } => {
                    let position = images
                        .iter()
                        .position(|image| image.item_id == item.id)
                        .ok_or_else(|| {
                            invalid(l10n!(
                                CATALOG,
                                language(),
                                "share-in-image-missing",
                                name = item.name.as_str()
                            ))
                        })?;
                    items.push(NewItem::Image(images.swap_remove(position).upload));
                }
                AttachmentKind::File => {
                    has_files = true;
                    items.push(NewItem::File {
                        source: UploadSource::Path {
                            path: item.path.to_string_lossy().into_owned(),
                        },
                        name: item.name,
                        mime_type: item.mime_type,
                    });
                }
            }
        }
        Ok(Outbound {
            token: share.token,
            items,
            has_files,
        })
    }

    /// 发出去了：记住房间，收起分享页。只有照片时文件已经存进附件区，目录马上删；有文件时目录留着给 tus 续传，
    /// 到期由 [`inbox::clean`] 删。
    pub(crate) fn sent(&self, token: &str, has_files: bool, room: &str) {
        let mut finished = None;
        self.pending.send_if_modified(|pending| {
            if pending.as_ref().is_some_and(|share| share.token == token) {
                finished = pending.take();
                true
            } else {
                false
            }
        });
        if let Some(share) = finished
            && !has_files
        {
            inbox::remove(&share.dir);
        }
        *self
            .last_room
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(room.to_owned());
        let remembered = Remembered {
            last_room: Some(room.to_owned()),
        };
        let path = self.root.join(LAST_TARGET_FILE);
        let written = serde_json::to_vec(&remembered)
            .map_err(std::io::Error::other)
            .and_then(|bytes| fs::write(&path, bytes));
        if let Err(error) = written {
            // 只影响下次默认选哪个聊天，不挡这次发送。
            tracing::warn!(%error, "could not remember the share target");
        }
    }

    /// 分享页：待发的分享加上此刻的去向。
    pub(crate) fn view(
        &self,
        pending: Option<&Claimed>,
        paired: bool,
        chat: &ChatViewModel,
        primary_bot: Option<&str>,
    ) -> ShareInViewModel {
        let Some(share) = pending else {
            return ShareInViewModel::default();
        };
        let last_room = self.last_room();
        ShareInViewModel {
            pending: Some(PendingShare {
                id: share.token.clone(),
                items: share
                    .items
                    .iter()
                    .map(|item| ShareInItem {
                        id: item.id.clone(),
                        kind: item.kind.clone(),
                        name: item.name.clone(),
                        mime_type: item.mime_type.clone(),
                        size: item.size,
                        path: item.path.to_string_lossy().into_owned(),
                    })
                    .collect(),
                text: share.text.clone(),
                destinations: view::destinations(paired, chat, primary_bot, last_room.as_deref()),
                dropped: share.dropped,
            }),
        }
    }
}
