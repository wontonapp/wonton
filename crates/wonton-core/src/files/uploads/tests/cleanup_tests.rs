//! 放好之后的清理与记录落盘：记录落盘前不清，删掉的上传不被清理放回来。

use std::os::unix::fs::MetadataExt;

use super::*;

/// 记录的目录项没落盘：断电后记录就没了，所以数据和待完成记录留着，等下次清理确认落盘了才清。
#[tokio::test]
async fn nothing_is_cleaned_up_until_the_record_is_durable() {
    let f = Fixture::new();
    let id = f
        .files
        .create_upload(new_upload(5), &any_langs())
        .await
        .unwrap();
    let data = f.uploads_dir().join(id.as_str());
    fs::write(&data, "hello").unwrap();
    let pending = f.dir.path().join(format!("files/pending/{id}.json"));

    // `fsync` 的错误只报告一次：第一次失败之后再 `fsync` 一次可能返回成功，所以重试要重新写一次记录，不能再问一次。
    crate::plugin::store::DIR_SYNC_FAILURES
        .lock()
        .unwrap()
        .insert(f.dir.path().join("files/records"), 2);
    assert!(f.files.finish_upload(&id, &any_langs()).await.is_ok());
    assert!(data.exists());
    assert!(pending.exists());
    f.files.sweep(now_secs()).await;
    assert!(data.exists(), "还是没落盘");

    f.files.sweep(now_secs()).await;
    assert!(!data.exists());
    assert!(!pending.exists());
}

/// 否则下次清理看到待完成记录和数据，会把主人删掉的文件再放回来。
#[tokio::test]
async fn a_deleted_upload_does_not_come_back_with_the_next_sweep() {
    let f = Fixture::new();
    let (id, data, temp) = placed_but_not_cleaned_up(&f, "hello").await;
    let visible = f.home_dir().join(TRANSFERS).join("a.txt");
    assert!(visible.exists());

    f.files
        .delete(&id, "phone".to_owned(), &any_langs())
        .await
        .unwrap();
    assert!(!visible.exists());
    assert!(!data.exists());
    assert!(!temp.exists());

    f.files.sweep(now_secs()).await;
    assert!(!visible.exists());
    assert_eq!(
        f.files.upload_state(&id).await.unwrap(),
        UploadState::Unknown
    );
}

/// 临时文件删不掉时待完成记录要留着，否则再也没人知道它；能删的时候下次清理接着清。
#[tokio::test]
async fn a_temporary_file_that_cannot_be_removed_keeps_its_pending_record() {
    let f = Fixture::new();
    let (id, data, temp) = placed_but_not_cleaned_up(&f, "hello").await;
    let transfers = f.home_dir().join(TRANSFERS);
    let aside = f.dir.path().join("transfers-aside");
    fs::rename(&transfers, &aside).unwrap();
    std::os::unix::fs::symlink(&aside, &transfers).unwrap();

    assert!(!f.files.forget_now(&id, false));
    let pending = f.dir.path().join(format!("files/pending/{id}.json"));
    assert!(pending.exists());

    fs::remove_file(&transfers).unwrap();
    fs::rename(&aside, &transfers).unwrap();
    f.files.sweep(now_secs()).await;
    assert!(!temp.exists());
    assert!(!data.exists());
    assert!(!pending.exists());
    assert_eq!(
        fs::metadata(transfers.join("a.txt")).unwrap().nlink(),
        1,
        "不再有第二个硬链接，删掉文件就释放空间"
    );
}

/// 主人改过的文件留着，补的记录没落盘时待完成记录不能删：断电后记录没了，文件就再也没人管。
#[tokio::test]
async fn a_kept_file_keeps_its_pending_record_until_the_record_is_durable() {
    let f = Fixture::new();
    let (id, _data, _temp) = placed_without_record(&f, "hello").await;
    let transfers = f.home_dir().join(TRANSFERS);
    fs::write(transfers.join("a.txt"), "edited by the owner").unwrap();
    let pending = f.dir.path().join(format!("files/pending/{id}.json"));
    fail_dir_sync(f.dir.path().join("files/records"), 2);

    abandon(&f, &id, "取消").await;
    assert!(pending.exists());
    assert!(matches!(
        f.files.registry.get(&id, now_secs()),
        Lookup::Found(_)
    ));
    abandon(&f, &id, "取消").await;
    assert!(pending.exists(), "fsync 的错误只报告一次，要重写一次才算");
    abandon(&f, &id, "取消").await;
    assert!(!pending.exists());
    assert!(transfers.join("a.txt").exists());
}
