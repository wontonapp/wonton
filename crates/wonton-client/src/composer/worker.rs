//! 一个附件的上传：照片传到 Matrix 的媒体库，文件交给文件服务、看传输列表。连不上时暂停，等连上了接着传；失败就停下等主人。

use std::sync::{Arc, Weak};

use wonton_l10n::l10n;
use wonton_protocol::{AttachmentFailure, AttachmentState, TransferState};

use super::ComposerLink;
use super::state::{Body, Item};
use super::uplink::{UplinkError, file_failure};
use crate::l10n::{CATALOG, language};

#[expect(
    clippy::cast_precision_loss,
    reason = "a progress ratio does not need every byte"
)]
fn ratio(done: u64, total: u64) -> Option<f32> {
    (total > 0).then(|| (done as f32 / total as f32).clamp(0.0, 1.0))
}

pub(super) async fn upload(link: Weak<ComposerLink>, room: String, id: String) {
    loop {
        let Some(link) = link.upgrade() else {
            return;
        };
        let Some(item) = link.lock().state.item(&room, &id).cloned() else {
            return;
        };
        let outcome = match &item.body {
            Body::Image { .. } => link.upload_image(&room, &item).await,
            Body::File { .. } => link.upload_file(&room, &item).await,
        };
        match outcome {
            Ok(()) => {}
            Err(UplinkError::Offline(reason)) => {
                tracing::info!(%reason, "composer upload paused until the connection is back");
                link.show(&room, &id, AttachmentState::Paused);
                let online = link.uplink.online();
                drop(link);
                online.await;
                continue;
            }
            Err(UplinkError::Failed(reason, message)) => {
                link.record(&room, |state| {
                    if let Some(item) = state.item_mut(&room, &id) {
                        item.state = AttachmentState::Failed { reason, message };
                    }
                });
            }
        }
        link.kick(&room);
        return;
    }
}

impl ComposerLink {
    async fn upload_image(self: &Arc<Self>, room: &str, item: &Item) -> Result<(), UplinkError> {
        let Body::Image {
            width,
            height,
            thumbnail,
            ..
        } = &item.body
        else {
            return Ok(());
        };
        self.show(
            room,
            &item.id,
            AttachmentState::Uploading { progress: None },
        );
        let local = |error: wonton_protocol::CoreError| {
            UplinkError::Failed(AttachmentFailure::LocalStorage, error.to_string())
        };
        let data = self.disk.image(&item.id).map_err(local)?;
        let thumbnail = match (thumbnail, self.disk.thumbnail(&item.id).map_err(local)?) {
            (Some(meta), Some(data)) => Some(wonton_matrix::ImageThumbnail {
                mime_type: meta.mime_type.clone(),
                data,
                width: meta.width,
                height: meta.height,
            }),
            _ => None,
        };
        let image = wonton_matrix::ImageUpload {
            filename: item.name.clone(),
            mime_type: item.mime_type.clone().unwrap_or_default(),
            data,
            width: *width,
            height: *height,
            thumbnail,
        };
        let progress = {
            let link = Arc::downgrade(self);
            let (room, id) = (room.to_owned(), item.id.clone());
            Box::new(move |done: u64, total: u64| {
                if let Some(link) = link.upgrade() {
                    let progress = ratio(done, total);
                    link.show(&room, &id, AttachmentState::Uploading { progress });
                }
            })
        };
        let uploaded = self
            .uplink
            .upload_image(room.to_owned(), image, progress)
            .await?;
        self.record(room, |state| {
            if let Some(item) = state.item_mut(room, &item.id) {
                if let Body::Image { uploaded: slot, .. } = &mut item.body {
                    *slot = Some(uploaded);
                }
                item.state = AttachmentState::Ready;
            }
        });
        Ok(())
    }

    async fn upload_file(self: &Arc<Self>, room: &str, item: &Item) -> Result<(), UplinkError> {
        let Body::File {
            source, transfer, ..
        } = &item.body
        else {
            return Ok(());
        };
        let transfer = match transfer {
            Some(transfer) => transfer.clone(),
            None => {
                let transfer = self.uplink.start_file(
                    source.clone(),
                    item.name.clone(),
                    item.mime_type.clone(),
                )?;
                self.record(room, |state| {
                    if let Some(Body::File { transfer: slot, .. }) =
                        state.item_mut(room, &item.id).map(|item| &mut item.body)
                    {
                        *slot = Some(transfer.clone());
                    }
                });
                transfer
            }
        };
        self.follow_transfer(room, &item.id, &transfer).await
    }

    /// 看着传输列表里的这次上传，把进展抄到附件上，直到传完或失败。
    async fn follow_transfer(
        self: &Arc<Self>,
        room: &str,
        id: &str,
        transfer: &str,
    ) -> Result<(), UplinkError> {
        let mut transfers = self.uplink.transfers();
        loop {
            let seen = transfers
                .borrow_and_update()
                .all()
                .find(|seen| seen.id == transfer)
                .cloned();
            let Some(seen) = seen else {
                return Err(UplinkError::Failed(
                    AttachmentFailure::Other,
                    l10n!(CATALOG, language(), "composer-transfer-gone"),
                ));
            };
            match seen.state {
                TransferState::Running => {
                    let progress = seen
                        .total_bytes
                        .and_then(|total| ratio(seen.transferred_bytes, total));
                    self.show(room, id, AttachmentState::Uploading { progress });
                }
                TransferState::Paused => self.show(room, id, AttachmentState::Paused),
                TransferState::Done | TransferState::LinkNotSent { .. } => {
                    let Some(file) = seen.file else {
                        return Err(UplinkError::Failed(
                            AttachmentFailure::Other,
                            l10n!(CATALOG, language(), "composer-transfer-gone"),
                        ));
                    };
                    self.record(room, |state| {
                        if let Some(item) = state.item_mut(room, id) {
                            if let Body::File { file: slot, .. } = &mut item.body {
                                *slot = Some(file);
                            }
                            item.size = seen.total_bytes.or(item.size);
                            item.state = AttachmentState::Ready;
                        }
                    });
                    return Ok(());
                }
                TransferState::Failed { reason, message } => {
                    // 文件服务自己会在连上后接着传；停在失败上的就是真失败了，等连接也没用。
                    return Err(match file_failure(reason, message) {
                        UplinkError::Offline(message) => {
                            UplinkError::Failed(AttachmentFailure::Upstream, message)
                        }
                        failed => failed,
                    });
                }
            }
            if transfers.changed().await.is_err() {
                // 文件服务停了：作用域在关，这个任务也快被停掉。
                return Ok(());
            }
        }
    }
}
