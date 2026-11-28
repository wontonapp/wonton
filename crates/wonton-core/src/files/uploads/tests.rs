use super::super::share_fs::UPLOAD_TEMP_PREFIX;

use super::*;
use crate::files::new_upload_id as new_id;
use crate::files::test_support::Fixture;
use crate::files::test_support::any_langs;

mod abandoning_tests;
mod cleanup_tests;
mod placement_tests;
mod reading_tests;

pub(super) fn new_upload(size: u64) -> NewUpload<'static> {
    NewUpload {
        id: Box::leak(Box::new(new_id())),
        name: Some("a.txt"),
        mime_type: None,
        size: Some(size),
        device: "phone",
    }
}

/// 放好了，没写记录（记录写失败了，比如磁盘满，或者崩溃了）：公开的名字、临时文件、数据、待完成记录都在。
pub(super) async fn placed_without_record(
    f: &Fixture,
    content: &str,
) -> (FileId, PathBuf, PathBuf) {
    let id = f
        .files
        .create_upload(new_upload(content.len() as u64), &any_langs())
        .await
        .unwrap();
    let data = f.uploads_dir().join(id.as_str());
    fs::write(&data, content).unwrap();
    let pending = f.files.uploads.load_pending(&id).unwrap().unwrap();
    f.files.place_marked(&pending, &any_langs()).unwrap();
    let transfers = f.home_dir().join(TRANSFERS);
    assert!(transfers.join("a.txt").exists());
    (
        id.clone(),
        data,
        transfers.join(format!("{UPLOAD_TEMP_PREFIX}{id}")),
    )
}

/// 崩溃在写好记录之后、清理之前：临时文件、数据、待完成记录都在。
pub(super) async fn placed_but_not_cleaned_up(
    f: &Fixture,
    content: &str,
) -> (FileId, PathBuf, PathBuf) {
    let id = f
        .files
        .create_upload(new_upload(content.len() as u64), &any_langs())
        .await
        .unwrap();
    let data = f.uploads_dir().join(id.as_str());
    fs::write(&data, content).unwrap();
    let task = id.clone();
    f.files
        .blocking(move |files| {
            let pending = files.uploads.load_pending(&task)?.unwrap();
            let (name, placed) = files.home_shares(&pending.username, false)?.place_upload(
                &files.uploads.dir,
                task.as_str(),
                pending.size,
                Names {
                    preferred: &pending.name,
                    owner: files.uploads.owner,
                    placing_inode: None,
                },
                &any_langs(),
                &mut |_, _| Ok(()),
            )?;
            files.registry.save(FileRecord {
                id: task.clone(),
                name: name.clone(),
                mime_type: None,
                declared_size: None,
                source: FileSource::Device,
                origin: Origin::Upload {
                    reference: ShareRef {
                        share: TRANSFERS.to_owned(),
                        path: vec![name],
                    },
                    placed,
                    username: pending.username.clone(),
                },
                access: [Accessor::Owner].into(),
                rooms: Default::default(),
                owner_granted: Default::default(),
                registered: None,
                created_at: now_secs(),
                expires_at: None,
            })?;
            Ok(())
        })
        .await
        .unwrap();
    let transfers = f.home_dir().join(TRANSFERS);
    (
        id.clone(),
        data,
        transfers.join(format!("{UPLOAD_TEMP_PREFIX}{id}")),
    )
}

pub(super) fn fail_dir_sync(dir: PathBuf, times: usize) {
    crate::plugin::store::DIR_SYNC_FAILURES
        .lock()
        .unwrap()
        .insert(dir, times);
}

/// 作废的三种方式。
pub(super) async fn abandon(f: &Fixture, id: &FileId, how: &str) {
    match how {
        "取消" => {
            let turn = f.files.upload_turn(id).await;
            f.files.upload_cancelled(&turn).await;
        }
        "tusd 没有它了" => {
            let turn = f.files.upload_turn(id).await;
            f.files.settle_lost_upload(&turn).await.unwrap();
        }
        _ => f.files.sweep(now_secs() + 2 * STALE_SECS).await,
    }
}
