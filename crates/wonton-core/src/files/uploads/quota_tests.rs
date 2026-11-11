//! 配额：`statvfs` 看不见它，创建和 tusd 回 500 时都要按配额判断放不放得下。

use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

use wonton_protocol::{FileFailure, FileId};

use super::super::new_id;
use super::super::test_support::{Fixture, any_langs};
use super::NewUpload;
use super::room::Quota;

/// 这个目录的配额替身：单元测试里造不出配额文件系统，`quota` 按目录取这里的值。
pub(super) static QUOTA: Mutex<Vec<(PathBuf, Quota)>> = Mutex::new(Vec::new());

struct QuotaOn(PathBuf);

impl QuotaOn {
    fn new(f: &Fixture, quota: Quota) -> Self {
        QUOTA.lock().unwrap().push((f.uploads_dir(), quota));
        Self(f.uploads_dir())
    }
}

impl Drop for QuotaOn {
    fn drop(&mut self) {
        QUOTA.lock().unwrap().retain(|(path, _)| *path != self.0);
    }
}

fn quota(space_left: u64, inodes_left: u64) -> Quota {
    Quota {
        space_limit: 10 << 20,
        space_left: Some(space_left),
        inodes_left: Some(inodes_left),
    }
}

fn new_upload(id: &FileId, size: u64) -> NewUpload<'_> {
    NewUpload {
        id,
        name: Some("a.txt"),
        mime_type: None,
        size: Some(size),
        device: "phone",
    }
}

/// 配额满时 statvfs 看着还有空地方，创建也要报 `storage_full`。
#[tokio::test]
async fn a_full_quota_has_no_room_even_with_disk_space_left() {
    let f = Fixture::new();
    let _on = QuotaOn::new(&f, quota(10, 100));
    let id = new_id();
    let failure = f
        .files
        .create_upload(new_upload(&id, 100), &any_langs())
        .await
        .unwrap_err();
    assert_eq!(failure.kind, FileFailure::StorageFull);
}

/// inode 配额用完了也一样。
#[tokio::test]
async fn an_exhausted_inode_quota_has_no_room() {
    let f = Fixture::new();
    let _on = QuotaOn::new(&f, quota(u64::MAX / 2, 0));
    let id = new_id();
    let failure = f
        .files
        .create_upload(new_upload(&id, 100), &any_langs())
        .await
        .unwrap_err();
    assert_eq!(failure.kind, FileFailure::StorageFull);
}

/// 配额还有地方就照常创建。
#[tokio::test]
async fn a_quota_with_room_does_not_block_creating() {
    let f = Fixture::new();
    let _on = QuotaOn::new(
        &f,
        Quota {
            space_limit: 10 << 20,
            space_left: Some(10 << 20),
            inodes_left: Some(10),
        },
    );
    let id = new_id();
    assert!(
        f.files
            .create_upload(new_upload(&id, 5), &any_langs())
            .await
            .is_ok()
    );
}

/// tusd 写到一半回 500：剩下的字节超过配额剩余就报 `storage_full`，配额还够就不管。
#[tokio::test]
async fn a_write_that_hits_the_quota_is_told_apart() {
    let f = Fixture::new();
    let id = new_id();
    f.files.pending_for_test(&id, 100, "phone");
    fs::write(f.uploads_dir().join(id.as_str()), vec![0u8; 10]).unwrap();
    let on = QuotaOn::new(&f, quota(50, 10));
    let failure = f.files.uploads.out_of_space(&id, &any_langs());
    assert_eq!(failure.unwrap().kind, FileFailure::StorageFull);

    drop(on);
    let _on = QuotaOn::new(&f, quota(u64::MAX / 2, 10));
    assert!(f.files.uploads.out_of_space(&id, &any_langs()).is_none());
}
