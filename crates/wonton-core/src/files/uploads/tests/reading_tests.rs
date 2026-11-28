//! 读上传的状态：放置与读取交错、tusd 丢了但文件被主人留下、目录里被改过的名字。

use super::*;

/// 第一次看记录时还没放好，看待完成记录时已经放好了：照实说放好了，否则客户端重传。
#[tokio::test]
async fn a_placement_finishing_while_asked_reads_as_finished() {
    let f = Fixture::new();
    let id = f
        .files
        .create_upload(new_upload(5), &any_langs())
        .await
        .unwrap();
    fs::write(f.uploads_dir().join(id.as_str()), "hello").unwrap();
    let state = f
        .files
        .state_now(&id, || {
            let pending = f.files.uploads.load_pending(&id).unwrap().unwrap();
            f.files.place_pending(pending, &any_langs()).unwrap();
        })
        .unwrap();
    assert_eq!(state, UploadState::Finished { size: 5 });
}

/// tusd 没有它了，放下的文件主人改过、留下了：如实说传完了，不让客户端重传。
#[tokio::test]
async fn a_lost_upload_whose_file_was_kept_reads_as_finished() {
    let f = Fixture::new();
    let (id, _data, _temp) = placed_without_record(&f, "hello").await;
    let transfers = f.home_dir().join(TRANSFERS);
    fs::write(transfers.join("a.txt"), "edited by the owner").unwrap();
    let turn = f.files.upload_turn(&id).await;
    assert_eq!(
        f.files.settle_lost_upload(&turn).await.unwrap(),
        UploadState::Finished { size: 5 },
        "tus 的长度创建后不变，主人改过也回声明的"
    );
}

/// 目录里现在的名字可能被改成了 bidi 覆盖：显示的名字清洗，引用保留原样才找得到文件。
#[tokio::test]
async fn a_name_found_in_the_directory_is_cleaned_for_display() {
    const TRICKY: &str = "report\u{202e}fdp.exe";
    let f = Fixture::new();
    let (id, _data, _temp) = placed_without_record(&f, "hello").await;
    let transfers = f.home_dir().join(TRANSFERS);
    fs::rename(transfers.join("a.txt"), transfers.join(TRICKY)).unwrap();
    let record = f.files.finish_upload(&id, &any_langs()).await.unwrap();
    assert_eq!(record.name, "reportfdp.exe");
    let Origin::Upload { reference, .. } = &record.origin else {
        panic!("{record:?}");
    };
    assert_eq!(reference.path, [TRICKY]);
    assert!(
        f.files
            .origin_shares(&record.origin)
            .unwrap()
            .open(reference)
            .is_ok()
    );

    // 作废时留下的也一样。
    let f = Fixture::new();
    let (id, _data, _temp) = placed_without_record(&f, "hello").await;
    let transfers = f.home_dir().join(TRANSFERS);
    fs::rename(transfers.join("a.txt"), transfers.join(TRICKY)).unwrap();
    abandon(&f, &id, "取消").await;
    let Lookup::Found(record) = f.files.registry.get(&id, now_secs()) else {
        panic!("主人改过名的文件要留着");
    };
    assert_eq!(record.name, "reportfdp.exe");
}
