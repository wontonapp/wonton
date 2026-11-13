//! 作废的上传：清到一半也不再放，放下的文件认准记下的 inode，没确认落盘就不算清完。

use std::fs;

use wonton_protocol::FileFailure;

use super::super::share_fs::TRANSFERS;
use super::super::share_fs::tests::DIR_SYNC_FAILS;
use super::tests::{new_upload, placed_without_record};
use super::*;
use crate::files::test_support::Fixture;
use crate::files::test_support::any_langs;

/// 作废时临时文件的目录没能落盘：待完成记录留着。清理这时看到完整的数据（比如崩溃在删数据之前），不能把作废的上传再放回去。
#[tokio::test]
async fn an_abandoned_upload_is_never_placed_again() {
    let f = Fixture::new();
    let (id, data, temp) = placed_without_record(&f, "hello").await;
    let temp_name = temp.file_name().unwrap().to_str().unwrap().to_owned();
    DIR_SYNC_FAILS.set(Some(temp_name));
    assert!(!f.files.abandon_now(&id), "还没清完");
    DIR_SYNC_FAILS.set(None);
    fs::write(&data, "hello").unwrap();
    assert!(
        f.files
            .uploads
            .load_pending(&id)
            .unwrap()
            .unwrap()
            .abandoned
    );

    assert_eq!(
        f.files.upload_state(&id).await.unwrap(),
        UploadState::Unknown,
        "客户端已经接不上了"
    );
    assert_eq!(
        f.files
            .finish_upload(&id, &any_langs())
            .await
            .unwrap_err()
            .kind,
        FileFailure::FileUnknown,
        "tusd 的钩子排在后面也一样"
    );
    let transfers = f.home_dir().join(TRANSFERS);
    assert!(!transfers.join("a.txt").exists());
    assert!(!transfers.join("a (1).txt").exists());
    assert!(!data.exists());
    assert!(f.files.uploads.load_pending(&id).unwrap().is_none());
}

/// 清理接着清，不放。数据没传完、也不算很久没有进度的，不用再等 24 小时。
#[tokio::test]
async fn the_sweep_finishes_an_abandoned_upload_without_placing_it() {
    let f = Fixture::new();
    let (id, data, _temp) = placed_without_record(&f, "hello").await;
    fs::remove_file(f.home_dir().join(TRANSFERS).join("a.txt")).unwrap();
    f.files.mark_abandoned(&id).unwrap();
    fs::write(&data, "he").unwrap();
    f.files.sweep(now_secs()).await;
    assert!(!f.home_dir().join(TRANSFERS).join("a.txt").exists());
    assert!(!data.exists());
    assert!(f.files.uploads.load_pending(&id).unwrap().is_none());
}

/// 上传的文件删了，目录没确认落盘：断电后文件可能回到公开的名字上，所以待完成记录留着，下次清理再判。审计写明。
#[tokio::test]
async fn a_removal_not_confirmed_on_disk_keeps_the_upload_for_the_next_sweep() {
    let f = Fixture::new();
    let (id, _data, _temp) = placed_without_record(&f, "hello").await;
    DIR_SYNC_FAILS.set(Some("a.txt".to_owned()));
    assert!(!f.files.abandon_now(&id));
    DIR_SYNC_FAILS.set(None);
    assert!(!f.home_dir().join(TRANSFERS).join("a.txt").exists());
    assert!(f.files.uploads.load_pending(&id).unwrap().is_some());
    let audited = f.audit_lines();
    assert_eq!(audited.last().unwrap()["outcome"], DELETED_UNSYNCED);

    f.files.sweep(now_secs()).await;
    assert!(f.files.uploads.load_pending(&id).unwrap().is_none());
}

/// 插件在核心删掉临时文件之后自己建一个，再硬链接到公开的名字上：不是记下的 inode，不补记录。
#[tokio::test]
async fn a_temporary_file_someone_planted_is_not_kept_as_the_upload() {
    let f = Fixture::new();
    let (id, data, temp) = placed_without_record(&f, "hello").await;
    let transfers = f.home_dir().join(TRANSFERS);
    fs::remove_file(transfers.join("a.txt")).unwrap();
    fs::remove_file(&temp).unwrap();
    fs::write(&temp, "planted").unwrap();
    fs::hard_link(&temp, transfers.join("invoice.pdf")).unwrap();

    assert!(f.files.abandon_now(&id));
    assert_eq!(
        f.files.upload_state(&id).await.unwrap(),
        UploadState::Unknown,
        "不补记录"
    );
    assert!(!temp.exists());
    assert!(!data.exists());
    assert_eq!(
        fs::read_to_string(transfers.join("invoice.pdf")).unwrap(),
        "planted",
        "那是插件的文件，不动"
    );
}

/// 插件也建不出能骗过重试的临时文件：放的时候不认它，放真正的数据。
#[tokio::test]
async fn placing_again_does_not_trust_a_planted_temporary_file() {
    let f = Fixture::new();
    let (id, _data, temp) = placed_without_record(&f, "hello").await;
    let transfers = f.home_dir().join(TRANSFERS);
    fs::remove_file(transfers.join("a.txt")).unwrap();
    fs::remove_file(&temp).unwrap();
    fs::write(&temp, "planted").unwrap();
    fs::hard_link(&temp, transfers.join("invoice.pdf")).unwrap();

    let record = f.files.finish_upload(&id, &any_langs()).await.unwrap();
    assert_eq!(record.name, "a.txt");
    assert_eq!(
        fs::read_to_string(transfers.join("a.txt")).unwrap(),
        "hello"
    );
}

/// 客户端走了之后 tusd 的钩子才到：不再记下这个上传，否则要白占 24 小时。
#[tokio::test]
async fn a_hook_that_arrives_after_the_client_left_is_refused() {
    let f = Fixture::new();
    let upload = new_upload(5);
    let turn = f.files.upload_turn(upload.id).await;
    f.files.upload_unclaimed(&turn).await;
    drop(turn);

    let refused = f
        .files
        .create_upload(upload, &any_langs())
        .await
        .unwrap_err();
    assert_eq!(refused.kind, FileFailure::InvalidInput);
    assert!(f.files.uploads.load_pending(upload.id).unwrap().is_none());
    assert!(
        f.files
            .create_upload(new_upload(5), &any_langs())
            .await
            .is_ok()
    );
}

/// 记下的编号不会一直留着。
#[test]
fn withdrawn_ids_do_not_pile_up() {
    let f = Fixture::new();
    let old = crate::files::new_upload_id();
    let uploads = &f.files.uploads;
    uploads.withdraw(&old);
    *uploads.withdrawn.lock().unwrap().get_mut(&old).unwrap() = 0;
    uploads.withdraw(&crate::files::new_upload_id());
    assert!(!uploads.take_withdrawn(&old));
    assert_eq!(uploads.withdrawn.lock().unwrap().len(), 1);
}
