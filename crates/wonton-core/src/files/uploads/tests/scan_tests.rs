//! 清理扫描：读不了的记录、损坏的记录、没进度的上传、没人等的临时文件。

use super::*;

/// 待完成目录里一项读不了，传完的上传照样放。
#[tokio::test]
async fn an_unreadable_entry_does_not_hold_up_finished_uploads() {
    let f = Fixture::new();
    let id = f
        .files
        .create_upload(new_upload(5), &any_langs())
        .await
        .unwrap();
    fs::write(f.uploads_dir().join(id.as_str()), "hello").unwrap();
    let pending = f.dir.path().join("files/pending");
    fs::create_dir(pending.join("broken.json")).unwrap();

    let scan = f.files.uploads.scan(now_secs());
    assert_eq!(scan.complete, [id]);
}

/// 损坏的记录只报一次错，改名留给主人看，它的数据不删。
#[tokio::test]
async fn a_corrupt_pending_record_is_set_aside_and_its_data_kept() {
    let f = Fixture::new();
    let id = f
        .files
        .create_upload(new_upload(5), &any_langs())
        .await
        .unwrap();
    let data = f.uploads_dir().join(id.as_str());
    fs::write(&data, "he").unwrap();
    let pending = f.dir.path().join(format!("files/pending/{id}.json"));
    fs::write(&pending, "{ not json").unwrap();

    let long_after = now_secs() + 2 * STALE_SECS;
    for _ in 0..2 {
        let scan = f.files.uploads.scan(long_after);
        assert!(scan.complete.is_empty());
    }
    assert!(!pending.exists());
    assert!(pending.with_extension("json.corrupt").exists());
    assert!(data.exists());
    assert_eq!(
        f.files.upload_state(&id).await.unwrap(),
        UploadState::Unknown
    );
}

#[tokio::test]
async fn an_upload_without_progress_is_cleared_with_its_temporary_copy() {
    let f = Fixture::new();
    let id = f
        .files
        .create_upload(new_upload(5), &any_langs())
        .await
        .unwrap();
    let data = f.uploads_dir().join(id.as_str());
    fs::write(&data, "he").unwrap();
    let temp = f
        .home_dir()
        .join(TRANSFERS)
        .join(format!("{UPLOAD_TEMP_PREFIX}{id}"));
    fs::create_dir_all(temp.parent().unwrap()).unwrap();
    fs::write(&temp, "he").unwrap();

    f.files.sweep(now_secs() + 2 * STALE_SECS).await;
    assert!(!data.exists());
    assert!(!temp.exists());
    assert_eq!(
        f.files.upload_state(&id).await.unwrap(),
        UploadState::Unknown
    );
}

#[tokio::test]
async fn a_sweep_removes_temporary_files_nobody_is_waiting_for() {
    let f = Fixture::new();
    f.files.home_shares("owner", true).unwrap();
    let transfers = f.home_dir().join(TRANSFERS);
    let orphan = transfers.join(format!("{UPLOAD_TEMP_PREFIX}{}", new_id()));
    fs::write(&orphan, "left behind").unwrap();
    // 还有待完成记录的：可能正要放。
    let id = f
        .files
        .create_upload(new_upload(5), &any_langs())
        .await
        .unwrap();
    let in_use = transfers.join(format!("{UPLOAD_TEMP_PREFIX}{id}"));
    fs::write(&in_use, "copying").unwrap();
    // 主人自己的文件，名字只是看着像。
    let owners = transfers.join(format!("{UPLOAD_TEMP_PREFIX}notes.txt"));
    fs::write(&owners, "the owner's").unwrap();
    // 插件放的目录，不是空的：这个名字只属于核心。
    let planted = transfers.join(format!("{UPLOAD_TEMP_PREFIX}{}", new_id()));
    fs::create_dir(&planted).unwrap();
    fs::write(planted.join("x"), "x").unwrap();

    f.files.sweep(now_secs()).await;
    assert!(!orphan.exists());
    assert!(!planted.exists());
    assert!(in_use.exists());
    assert!(owners.exists());
}
