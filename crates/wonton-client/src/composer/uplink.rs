//! 附件区往外走的几条路：照片传到 Matrix、文件经文件服务、授权、把一组事件交给发送队列。行为测试换成测试替身。

use std::sync::Arc;
use std::time::Duration;

use futures_util::future::BoxFuture;
use serde_json::Value;
use tokio::sync::watch;
use wonton_l10n::l10n;
use wonton_matrix::{GroupPart, ImageUpload, OwnedEventId, OwnedRoomId, UploadedImage};
use wonton_protocol::{
    AttachmentFailure, ChatAccount, CoreError, FileFailure, FileId, HomeStatus, TransfersViewModel,
    UploadSource,
};

use crate::chat::ChatLink;
use crate::files::FilesLink;
use crate::home::HomeLink;
use crate::l10n::{CATALOG, language};

/// 上传或发送没做成。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum UplinkError {
    /// 连不上、还没登录：等 [`Uplink::online`] 再试，不算失败。
    Offline(String),
    /// 有明确原因的失败，再试也一样，等主人处理。
    Failed(AttachmentFailure, String),
}

/// 传上去的照片：`handle` 是交回 [`Uplink::send_group`] 的不透明内容，可以存盘。
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub(crate) struct Uploaded {
    pub(crate) handle: Value,
    pub(crate) encrypted: bool,
}

/// 一组里的一条事件。
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Part {
    Image {
        uploaded: Uploaded,
        caption: Option<String>,
    },
    Text(String),
}

/// 交出一组事件的请求。
pub(crate) struct Handover {
    pub(crate) room: String,
    pub(crate) parts: Vec<Part>,
    pub(crate) group_id: Option<String>,
    pub(crate) reply_to: Option<String>,
    /// 前面已经交出去的条数，这次从它后面接着交。
    pub(crate) skip: usize,
}

pub(crate) type Progress = Box<dyn Fn(u64, u64) + Send + Sync>;
pub(crate) type Handed = Box<dyn Fn(usize) + Send + Sync>;

pub(crate) trait Uplink: Send + Sync + 'static {
    /// 点发送前的检查：房间能发、回复对象合法。
    fn check_send(&self, room: &str, reply_to: Option<&str>) -> Result<(), CoreError>;
    /// 点了发送，草稿用掉了。
    fn forget_draft(&self, room: &str, body: &str);
    /// 一组交给了发送队列。
    fn sent(&self, room: &str);
    fn upload_image(
        &self,
        room: String,
        image: ImageUpload,
        progress: Progress,
    ) -> BoxFuture<'static, Result<Uploaded, UplinkError>>;
    fn room_is_encrypted(&self, room: String) -> BoxFuture<'static, Result<bool, UplinkError>>;
    fn start_file(
        &self,
        source: UploadSource,
        name: String,
        mime_type: Option<String>,
    ) -> Result<String, UplinkError>;
    fn drop_file(&self, transfer: &str);
    /// 传输列表：文件的进展看它。
    fn transfers(&self) -> watch::Receiver<TransfersViewModel>;
    fn grant(&self, file: FileId, room: String) -> BoxFuture<'static, Result<(), UplinkError>>;
    /// 按顺序把 `skip` 之后的事件交给发送队列，每交出一条调一次 `handed`（交出去的总条数）。
    fn send_group(
        &self,
        handover: Handover,
        handed: Handed,
    ) -> BoxFuture<'static, Result<(), UplinkError>>;
    /// 等到可能又连得上了。
    fn online(&self) -> BoxFuture<'static, ()>;
}

/// 连不上之后至少隔多久再试：连接看起来是好的、请求却一直失败时，别不停地重试。
const RETRY: Duration = Duration::from_secs(5);

pub(crate) struct LiveUplink {
    pub(crate) chat: Arc<ChatLink>,
    pub(crate) files: Arc<crate::files::FilesLink>,
    pub(crate) home: Arc<HomeLink>,
}

impl LiveUplink {
    fn session(&self) -> Result<Arc<wonton_matrix::MatrixSession>, UplinkError> {
        self.chat
            .current()
            .map_err(|error| UplinkError::Offline(error.to_string()))
    }
}

fn room_id(room: &str) -> Result<OwnedRoomId, UplinkError> {
    room.parse()
        .map_err(|_| UplinkError::Failed(AttachmentFailure::Rejected, room.to_owned()))
}

/// Matrix 这一侧的错误算哪一类。
fn matrix_error(error: &wonton_matrix::Error) -> UplinkError {
    use wonton_matrix::Error;
    let message = error.to_string();
    match error {
        Error::Transport(_) | Error::NotReady(_) => UplinkError::Offline(message),
        // 类型在放进附件区时就核对过，这里的输入非法只剩比服务器的上传上限大。
        Error::InvalidInput(_) => UplinkError::Failed(AttachmentFailure::TooLarge, message),
        Error::Unauthorized { .. } | Error::Forbidden { .. } => {
            UplinkError::Failed(AttachmentFailure::Rejected, message)
        }
        Error::Upstream(_) => UplinkError::Failed(AttachmentFailure::Upstream, message),
    }
}

/// 文件服务的失败类别算附件的哪一类；连不上、服务暂时不可用算等连接。
pub(crate) fn file_failure(reason: FileFailure, message: String) -> UplinkError {
    let failure = match reason {
        FileFailure::TransportUnavailable | FileFailure::ServiceUnavailable => {
            return UplinkError::Offline(message);
        }
        FileFailure::LocalSourceUnavailable | FileFailure::LocalPermissionDenied => {
            AttachmentFailure::SourceUnavailable
        }
        FileFailure::StorageFull => AttachmentFailure::StorageFull,
        FileFailure::LocalIo | FileFailure::LocalStorageFull => AttachmentFailure::LocalStorage,
        FileFailure::Unauthenticated
        | FileFailure::NotShared
        | FileFailure::PermissionDenied
        | FileFailure::InvalidInput => AttachmentFailure::Rejected,
        FileFailure::Other => AttachmentFailure::Other,
        _ => AttachmentFailure::Upstream,
    };
    UplinkError::Failed(failure, message)
}

impl Uplink for LiveUplink {
    fn check_send(&self, room: &str, reply_to: Option<&str>) -> Result<(), CoreError> {
        self.chat.check_send(room, reply_to)
    }

    fn forget_draft(&self, room: &str, body: &str) {
        self.chat.forget_sent_draft(room, body);
    }

    fn sent(&self, room: &str) {
        self.chat.record_sent_room(room);
    }

    fn upload_image(
        &self,
        room: String,
        image: ImageUpload,
        progress: Progress,
    ) -> BoxFuture<'static, Result<Uploaded, UplinkError>> {
        let session = self.session();
        Box::pin(async move {
            let room = room_id(&room)?;
            let uploaded = session?
                .upload_image(&room, image, progress)
                .await
                .map_err(|error| matrix_error(&error))?;
            Ok(Uploaded {
                encrypted: uploaded.is_encrypted(),
                handle: serde_json::to_value(&uploaded).expect("uploaded images serialize"),
            })
        })
    }

    fn room_is_encrypted(&self, room: String) -> BoxFuture<'static, Result<bool, UplinkError>> {
        let session = self.session();
        Box::pin(async move {
            session?
                .room_is_encrypted(&room_id(&room)?)
                .await
                .map_err(|error| matrix_error(&error))
        })
    }

    fn start_file(
        &self,
        source: UploadSource,
        name: String,
        mime_type: Option<String>,
    ) -> Result<String, UplinkError> {
        self.files
            .start_held_upload(source, name, mime_type)
            .map_err(|error| match error {
                CoreError::NotReady { reason } | CoreError::TransportUnavailable { reason } => {
                    UplinkError::Offline(reason)
                }
                CoreError::Storage { reason } => {
                    UplinkError::Failed(AttachmentFailure::LocalStorage, reason)
                }
                other => UplinkError::Failed(AttachmentFailure::Rejected, other.to_string()),
            })
    }

    fn drop_file(&self, transfer: &str) {
        self.files.drop_held_upload(transfer);
    }

    fn transfers(&self) -> watch::Receiver<TransfersViewModel> {
        self.files.sender().subscribe()
    }

    fn grant(&self, file: FileId, room: String) -> BoxFuture<'static, Result<(), UplinkError>> {
        let files: Arc<FilesLink> = self.files.clone();
        Box::pin(async move {
            files
                .grant_to_room(&file, &room)
                .await
                .map_err(|(reason, message)| file_failure(reason, message))
        })
    }

    fn send_group(
        &self,
        handover: Handover,
        handed: Handed,
    ) -> BoxFuture<'static, Result<(), UplinkError>> {
        let session = self.session();
        Box::pin(async move {
            let room = room_id(&handover.room)?;
            let reply_to = handover
                .reply_to
                .map(|id| id.parse::<OwnedEventId>())
                .transpose()
                .map_err(|error| {
                    UplinkError::Failed(AttachmentFailure::Rejected, error.to_string())
                })?;
            let parts = handover
                .parts
                .into_iter()
                .map(group_part)
                .collect::<Result<Vec<_>, _>>()?;
            session?
                .send_group(
                    &room,
                    parts,
                    handover.group_id.as_deref(),
                    reply_to.as_deref(),
                    handover.skip,
                    handed,
                )
                .await
                .map_err(|error| matrix_error(&error))
        })
    }

    fn online(&self) -> BoxFuture<'static, ()> {
        let mut chat = self.chat.subscribe();
        let mut home = self.home.subscribe();
        Box::pin(async move {
            tokio::time::sleep(RETRY).await;
            let _ = home
                .wait_for(|view| view.status == HomeStatus::Connected)
                .await;
            let _ = chat
                .wait_for(|view| matches!(view.account, ChatAccount::SignedIn { .. }))
                .await;
        })
    }
}

fn group_part(part: Part) -> Result<GroupPart, UplinkError> {
    Ok(match part {
        Part::Text(body) => GroupPart::Text { body },
        Part::Image { uploaded, caption } => GroupPart::Image {
            image: serde_json::from_value::<UploadedImage>(uploaded.handle).map_err(|error| {
                UplinkError::Failed(
                    AttachmentFailure::LocalStorage,
                    l10n!(
                        CATALOG,
                        language(),
                        "composer-upload-record-unreadable",
                        reason = error.to_string()
                    ),
                )
            })?,
            caption,
        },
    })
}
