//! 待完成记录：没传完的上传、链接之前的名字和样子，放好之后用它写记录。

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use wonton_protocol::{FileId, FileSource};

use super::super::record::{Accessor, FileRecord, Origin};
use super::super::share_fs::{ShareRef, Snapshot, TRANSFERS, sanitize_name};
use super::{Failure, UPLOAD_TTL_SECS, Uploads};
use crate::plugin::manager::now_secs;
use crate::plugin::store::read_json;

/// 还没传完的上传。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(in crate::files) struct Pending {
    pub(super) id: FileId,
    pub(super) name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) mime_type: Option<String>,
    pub(super) size: u64,
    pub(super) device: String,
    pub(super) username: String,
    pub(super) accessor: Accessor,
    pub(super) created_at: u64,
    /// 要放到哪个名字上、文件这时的样子，链接之前写好。作废时靠它分清名字上的文件是放下时的样子，还是主人后来改过的。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) placing: Option<Placing>,
    /// 作废了，清理到一半（崩溃了，或者有东西删不掉）。数据可能还是完整的，也不能再放：客户端已经被告知要重传，放了就多一份。
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub(super) abandoned: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(in crate::files) struct Placing {
    pub(super) name: String,
    pub(super) snapshot: Snapshot,
}

impl Pending {
    /// 放在 `name` 上的文件的记录。`name` 是目录里现在的名字，主人和插件都能改，显示给人看的要和别的不可信的名字一样清洗；引用保留原样，
    /// 否则找不到文件。链接没发出去之前一直带过期时间，授权给房间或者发到设备时清掉。
    pub(super) fn record(&self, name: String, placed: Snapshot) -> FileRecord {
        FileRecord {
            id: self.id.clone(),
            name: sanitize_name(&name),
            mime_type: self.mime_type.clone(),
            declared_size: Some(self.size),
            source: FileSource::Device,
            origin: Origin::Upload {
                reference: ShareRef {
                    share: TRANSFERS.to_owned(),
                    path: vec![name],
                },
                placed,
                username: self.username.clone(),
            },
            access: [self.accessor.clone()].into(),
            rooms: Default::default(),
            owner_granted: Default::default(),
            registered: None,
            created_at: now_secs(),
            expires_at: Some(now_secs().saturating_add(UPLOAD_TTL_SECS)),
        }
    }
}

impl Uploads {
    pub(super) fn pending_path(&self, id: &FileId) -> PathBuf {
        self.pending.join(format!("{id}.json"))
    }

    pub(super) fn load_pending(&self, id: &FileId) -> Result<Option<Pending>, Failure> {
        read_json(&self.pending_path(id)).map_err(|e| Failure::internal(e.to_string()))
    }
}
