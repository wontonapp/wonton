//! 作废的上传：三种作废方式都不留公开文件，主人接手过的文件留着。

use super::*;

const ABANDON: [&str; 3] = ["取消", "tusd 没有它了", "过期"];

async fn assert_nothing_left(f: &Fixture, id: &FileId, data: &Path, temp: &Path) {
    assert!(
        !f.home_dir().join(TRANSFERS).join("a.txt").exists(),
        "公开的文件"
    );
    assert!(!temp.exists(), "临时文件");
    assert!(!data.exists(), "数据");
    assert_eq!(
        f.files.upload_state(id).await.unwrap(),
        UploadState::Unknown
    );
    assert_eq!(
        visible_transfers(f),
        [".wonton-trash"],
        "回收槽是核心预先建好的"
    );
}

fn visible_transfers(f: &Fixture) -> Vec<String> {
    fs::read_dir(f.home_dir().join(TRANSFERS))
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect()
}

#[tokio::test]
async fn cancelling_leaves_no_public_file_without_a_record() {
    let f = Fixture::new();
    let (id, data, temp) = placed_without_record(&f, "hello").await;
    fs::remove_file(&data).unwrap(); // tusd 回 204 之前删了。
    let turn = f.files.upload_turn(&id).await;
    f.files.upload_cancelled(&turn).await;
    drop(turn);
    assert_nothing_left(&f, &id, &data, &temp).await;
    let line = f.audit_lines().pop().expect("删除要进审计");
    assert_eq!(line["event"], "file_delete");
    assert_eq!(line["file"], id.as_str());
    assert_eq!(line["device"], "phone");
    assert_eq!(line["outcome"], "ok");
}

#[tokio::test]
async fn an_upload_tusd_lost_leaves_no_public_file_without_a_record() {
    let f = Fixture::new();
    let (id, data, temp) = placed_without_record(&f, "hello").await;
    fs::remove_file(&data).unwrap();
    let turn = f.files.upload_turn(&id).await;
    let settled = f.files.settle_lost_upload(&turn).await.unwrap();
    assert_eq!(settled, UploadState::Unknown);
    drop(turn);
    assert_nothing_left(&f, &id, &data, &temp).await;
}

#[tokio::test]
async fn a_stale_upload_leaves_no_public_file_without_a_record() {
    let f = Fixture::new();
    let (id, data, temp) = placed_without_record(&f, "hello").await;
    fs::remove_file(&data).unwrap();
    f.files.sweep(now_secs() + 2 * STALE_SECS).await;
    assert_nothing_left(&f, &id, &data, &temp).await;
}

/// 名字上已经是主人别的文件：不动它。
#[tokio::test]
async fn abandoning_leaves_what_the_owner_put_on_the_name_alone() {
    let f = Fixture::new();
    let (id, data, temp) = placed_without_record(&f, "hello").await;
    let public = f.home_dir().join(TRANSFERS).join("a.txt");
    fs::remove_file(&public).unwrap();
    fs::write(&public, "the owner's").unwrap();
    fs::remove_file(&data).unwrap();
    let turn = f.files.upload_turn(&id).await;
    f.files.upload_cancelled(&turn).await;
    drop(turn);
    assert_eq!(fs::read_to_string(&public).unwrap(), "the owner's");
    assert!(!temp.exists());
    assert!(
        matches!(f.files.registry.get(&id, now_secs()), Lookup::Unknown),
        "不是这个上传的文件，不补记录"
    );
    assert!(f.audit_lines().is_empty(), "没删东西");
}

/// 写记录失败以后，主人在 Samba 里改了名、改了内容：同一个 inode，事后取的快照总对得上，只有放下时记的样子才分得清。
#[tokio::test]
async fn a_file_the_owner_renamed_and_edited_survives_abandoning() {
    for how in ABANDON {
        let f = Fixture::new();
        let (id, data, temp) = placed_without_record(&f, "hello").await;
        let transfers = f.home_dir().join(TRANSFERS);
        fs::rename(transfers.join("a.txt"), transfers.join("vacation.jpg")).unwrap();
        fs::write(transfers.join("vacation.jpg"), "hello, with EXIF").unwrap();
        fs::remove_file(&data).unwrap();

        abandon(&f, &id, how).await;

        let kept = transfers.join("vacation.jpg");
        assert_eq!(fs::read_to_string(&kept).unwrap(), "hello, with EXIF");
        assert!(!temp.exists(), "{how}");
        let Lookup::Found(record) = f.files.registry.get(&id, now_secs()) else {
            panic!("{how}：主人的文件要有记录，App 才删得掉");
        };
        assert_eq!(record.name, "vacation.jpg");
        assert!(f.audit_lines().is_empty(), "{how}：没删东西");
        // 记录是对的：App 能删掉它。
        assert!(
            f.files
                .delete(&id, "phone".to_owned(), &any_langs())
                .await
                .is_ok(),
            "{how}"
        );
        assert!(!kept.exists());
    }
}

#[tokio::test]
async fn a_file_only_renamed_or_only_edited_is_the_owners_too() {
    for how in ABANDON {
        let f = Fixture::new();
        let (id, data, _) = placed_without_record(&f, "hello").await;
        let transfers = f.home_dir().join(TRANSFERS);
        fs::rename(transfers.join("a.txt"), transfers.join("b.txt")).unwrap();
        fs::remove_file(&data).unwrap();
        abandon(&f, &id, how).await;
        assert_eq!(
            fs::read_to_string(transfers.join("b.txt")).unwrap(),
            "hello"
        );
        assert!(matches!(
            f.files.registry.get(&id, now_secs()),
            Lookup::Found(_)
        ));

        let f = Fixture::new();
        let (id, data, _) = placed_without_record(&f, "hello").await;
        let transfers = f.home_dir().join(TRANSFERS);
        fs::write(transfers.join("a.txt"), "edited").unwrap();
        fs::remove_file(&data).unwrap();
        abandon(&f, &id, how).await;
        assert_eq!(
            fs::read_to_string(transfers.join("a.txt")).unwrap(),
            "edited"
        );
        assert!(matches!(
            f.files.registry.get(&id, now_secs()),
            Lookup::Found(_)
        ));
    }
}

/// 记下了，崩溃在链接之前：名字空着，上面后来有了别的文件也不是这个上传的。
#[tokio::test]
async fn a_crash_before_the_link_leaves_nothing_to_settle() {
    let f = Fixture::new();
    let id = f
        .files
        .create_upload(new_upload(5), &any_langs())
        .await
        .unwrap();
    let data = f.uploads_dir().join(id.as_str());
    fs::write(&data, "hello").unwrap();
    let mut pending = f.files.uploads.load_pending(&id).unwrap().unwrap();
    pending.placing = Some(Placing {
        name: "a.txt".to_owned(),
        snapshot: Snapshot {
            inode: 1,
            size: 5,
            modified_ns: 1,
        },
    });
    write_json(&f.files.uploads.pending_path(&id), &pending).unwrap();
    let public = f.home_dir().join(TRANSFERS).join("a.txt");
    fs::create_dir_all(public.parent().unwrap()).unwrap();
    fs::write(&public, "the owner's").unwrap();

    let turn = f.files.upload_turn(&id).await;
    f.files.upload_cancelled(&turn).await;
    drop(turn);
    assert_eq!(fs::read_to_string(&public).unwrap(), "the owner's");
    assert!(matches!(
        f.files.registry.get(&id, now_secs()),
        Lookup::Unknown
    ));
}

/// 有记录的上传，公开的文件是主人的：清掉残留时不能跟着删。
#[tokio::test]
async fn abandoning_keeps_a_file_that_has_a_record() {
    let f = Fixture::new();
    let (id, data, temp) = placed_but_not_cleaned_up(&f, "hello").await;
    fs::remove_file(&data).unwrap();
    f.files.sweep(now_secs() + 2 * STALE_SECS).await;
    assert!(f.home_dir().join(TRANSFERS).join("a.txt").exists());
    assert!(!temp.exists());
    assert_eq!(
        f.files.upload_state(&id).await.unwrap(),
        UploadState::Finished { size: 5 }
    );
}
