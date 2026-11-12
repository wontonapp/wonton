use super::super::share_fs::UPLOAD_TEMP_PREFIX;
use std::os::unix::fs::MetadataExt;
use std::time::Duration;

use super::*;
use crate::files::new_upload_id as new_id;
use crate::files::test_support::Fixture;

pub(super) fn new_upload(size: u64) -> NewUpload<'static> {
    NewUpload {
        id: Box::leak(Box::new(new_id())),
        name: Some("a.txt"),
        mime_type: None,
        size: Some(size),
        device: "phone",
    }
}

/// 还有人在排队时离开，后来的调用要和他们排同一个队，不能拿到一把新锁并发放同一个上传。
#[tokio::test]
async fn a_turn_someone_is_waiting_for_stays_in_the_table() {
    let f = Fixture::new();
    let uploads = &f.files.uploads;
    let id = new_id();

    let first = uploads.turn(&id).await;
    let mut waiting = Box::pin(uploads.turn(&id));
    assert!(futures_util::poll!(&mut waiting).is_pending());
    drop(first);
    let waiting = waiting.await;
    let mut later = Box::pin(uploads.turn(&id));
    assert!(
        futures_util::poll!(&mut later).is_pending(),
        "要和拿着锁的排同一个队"
    );
    drop(waiting);
    drop(later.await);
    assert!(uploads.finishing.lock().unwrap().is_empty());

    // 请求断了（排队的 future 被丢掉）也不留在表里。
    let held = uploads.turn(&id).await;
    let mut gave_up = Box::pin(uploads.turn(&id));
    assert!(futures_util::poll!(&mut gave_up).is_pending());
    drop(gave_up);
    drop(held);
    assert!(uploads.finishing.lock().unwrap().is_empty());
}

/// 待完成目录里一项读不了，传完的上传照样放。
#[tokio::test]
async fn an_unreadable_entry_does_not_hold_up_finished_uploads() {
    let f = Fixture::new();
    let id = f.files.create_upload(new_upload(5)).await.unwrap();
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
    let id = f.files.create_upload(new_upload(5)).await.unwrap();
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
    let id = f.files.create_upload(new_upload(5)).await.unwrap();
    let data = f.uploads_dir().join(id.as_str());
    fs::write(&data, "he").unwrap();
    let temp = f
        .shares_dir
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

/// 放好了，没写记录（记录写失败了，比如磁盘满，或者崩溃了）：公开的名字、临时文件、数据、待完成记录都在。
pub(super) async fn placed_without_record(
    f: &Fixture,
    content: &str,
) -> (FileId, PathBuf, PathBuf) {
    let id = f
        .files
        .create_upload(new_upload(content.len() as u64))
        .await
        .unwrap();
    let data = f.uploads_dir().join(id.as_str());
    fs::write(&data, content).unwrap();
    let pending = f.files.uploads.load_pending(&id).unwrap().unwrap();
    f.files.place_marked(&pending).unwrap();
    let transfers = f.shares_dir.join(TRANSFERS);
    assert!(transfers.join("a.txt").exists());
    (
        id.clone(),
        data,
        transfers.join(format!("{UPLOAD_TEMP_PREFIX}{id}")),
    )
}

async fn assert_nothing_left(f: &Fixture, id: &FileId, data: &Path, temp: &Path) {
    assert!(
        !f.shares_dir.join(TRANSFERS).join("a.txt").exists(),
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
    fs::read_dir(f.shares_dir.join(TRANSFERS))
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
    let public = f.shares_dir.join(TRANSFERS).join("a.txt");
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

/// 作废的三种方式。
async fn abandon(f: &Fixture, id: &FileId, how: &str) {
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

const ABANDON: [&str; 3] = ["取消", "tusd 没有它了", "过期"];

/// 写记录失败以后，主人在 Samba 里改了名、改了内容：同一个 inode，事后取的快照总对得上，只有放下时记的样子才分得清。
#[tokio::test]
async fn a_file_the_owner_renamed_and_edited_survives_abandoning() {
    for how in ABANDON {
        let f = Fixture::new();
        let (id, data, temp) = placed_without_record(&f, "hello").await;
        let transfers = f.shares_dir.join(TRANSFERS);
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
            f.files.delete(&id, "phone".to_owned()).await.is_ok(),
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
        let transfers = f.shares_dir.join(TRANSFERS);
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
        let transfers = f.shares_dir.join(TRANSFERS);
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

/// 名字和样子要在链接之前记下：链接之后崩溃，记录里已经有了。重名时记的是真正要放的那个名字。
#[tokio::test]
async fn the_name_and_look_are_written_before_the_link() {
    let f = Fixture::new();
    let id = f.files.create_upload(new_upload(5)).await.unwrap();
    fs::write(f.uploads_dir().join(id.as_str()), "hello").unwrap();
    let transfers = f.shares_dir.join(TRANSFERS);
    fs::create_dir_all(&transfers).unwrap();
    fs::write(transfers.join("a.txt"), "the owner's").unwrap();

    let mut seen = Vec::new();
    let (name, placed) = f
        .files
        .shares
        .place_upload(
            &f.files.uploads.dir,
            id.as_str(),
            5,
            Names {
                preferred: "a.txt",
                owner: f.files.uploads.owner,
                placing_inode: None,
            },
            &mut |name, snapshot| {
                assert!(!transfers.join(name).exists(), "链接之前");
                seen.push((name.to_owned(), *snapshot));
                Ok(())
            },
        )
        .unwrap();
    assert_eq!(seen, [(name.clone(), placed)]);
    assert_eq!(name, "a (1).txt");

    // 经 `place_marked` 走的话，记在待完成记录里。
    let id = f.files.create_upload(new_upload(5)).await.unwrap();
    fs::write(f.uploads_dir().join(id.as_str()), "hello").unwrap();
    let pending = f.files.uploads.load_pending(&id).unwrap().unwrap();
    let (name, placed) = f.files.place_marked(&pending).unwrap();
    let pending = f.files.uploads.load_pending(&id).unwrap().unwrap();
    assert_eq!(
        pending.placing,
        Some(Placing {
            name,
            snapshot: placed
        })
    );
}

/// 记下了，崩溃在链接之前：名字空着，上面后来有了别的文件也不是这个上传的。
#[tokio::test]
async fn a_crash_before_the_link_leaves_nothing_to_settle() {
    let f = Fixture::new();
    let id = f.files.create_upload(new_upload(5)).await.unwrap();
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
    let public = f.shares_dir.join(TRANSFERS).join("a.txt");
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
    assert!(f.shares_dir.join(TRANSFERS).join("a.txt").exists());
    assert!(!temp.exists());
    assert_eq!(
        f.files.upload_state(&id).await.unwrap(),
        UploadState::Finished { size: 5 }
    );
}

/// 第一次看记录时还没放好，看待完成记录时已经放好了：照实说放好了，否则客户端重传。
#[tokio::test]
async fn a_placement_finishing_while_asked_reads_as_finished() {
    let f = Fixture::new();
    let id = f.files.create_upload(new_upload(5)).await.unwrap();
    fs::write(f.uploads_dir().join(id.as_str()), "hello").unwrap();
    let state = f
        .files
        .state_now(&id, || {
            let pending = f.files.uploads.load_pending(&id).unwrap().unwrap();
            f.files.place_pending(pending).unwrap();
        })
        .unwrap();
    assert_eq!(state, UploadState::Finished { size: 5 });
}

/// 记录的目录项没落盘：断电后记录就没了，所以数据和待完成记录留着，等下次清理确认落盘了才清。
#[tokio::test]
async fn nothing_is_cleaned_up_until_the_record_is_durable() {
    let f = Fixture::new();
    let id = f.files.create_upload(new_upload(5)).await.unwrap();
    let data = f.uploads_dir().join(id.as_str());
    fs::write(&data, "hello").unwrap();
    let pending = f.dir.path().join(format!("files/pending/{id}.json"));

    // `fsync` 的错误只报告一次：第一次失败之后再 `fsync` 一次可能返回成功，所以重试要重新写一次记录，不能再问一次。
    crate::plugin::store::DIR_SYNC_FAILURES
        .lock()
        .unwrap()
        .insert(f.dir.path().join("files/records"), 2);
    assert!(f.files.finish_upload(&id).await.is_ok());
    assert!(data.exists());
    assert!(pending.exists());
    f.files.sweep(now_secs()).await;
    assert!(data.exists(), "还是没落盘");

    f.files.sweep(now_secs()).await;
    assert!(!data.exists());
    assert!(!pending.exists());
}

/// 崩溃在写好记录之后、清理之前：临时文件、数据、待完成记录都在。
async fn placed_but_not_cleaned_up(f: &Fixture, content: &str) -> (FileId, PathBuf, PathBuf) {
    let id = f
        .files
        .create_upload(new_upload(content.len() as u64))
        .await
        .unwrap();
    let data = f.uploads_dir().join(id.as_str());
    fs::write(&data, content).unwrap();
    let task = id.clone();
    f.files
        .blocking(move |files| {
            let pending = files.uploads.load_pending(&task)?.unwrap();
            let (name, placed) = files.shares.place_upload(
                &files.uploads.dir,
                task.as_str(),
                pending.size,
                Names {
                    preferred: &pending.name,
                    owner: files.uploads.owner,
                    placing_inode: None,
                },
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
                },
                access: [Accessor::Owner].into(),
                owner_granted: Default::default(),
                registered: None,
                created_at: now_secs(),
                expires_at: None,
            })?;
            Ok(())
        })
        .await
        .unwrap();
    let transfers = f.shares_dir.join(TRANSFERS);
    (
        id.clone(),
        data,
        transfers.join(format!("{UPLOAD_TEMP_PREFIX}{id}")),
    )
}

/// 否则下次清理看到待完成记录和数据，会把主人删掉的文件再放回来。
#[tokio::test]
async fn a_deleted_upload_does_not_come_back_with_the_next_sweep() {
    let f = Fixture::new();
    let (id, data, temp) = placed_but_not_cleaned_up(&f, "hello").await;
    let visible = f.shares_dir.join(TRANSFERS).join("a.txt");
    assert!(visible.exists());

    f.files.delete(&id, "phone".to_owned()).await.unwrap();
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
    let transfers = f.shares_dir.join(TRANSFERS);
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

#[tokio::test]
async fn a_sweep_removes_temporary_files_nobody_is_waiting_for() {
    let f = Fixture::new();
    let transfers = f.shares_dir.join(TRANSFERS);
    let orphan = transfers.join(format!("{UPLOAD_TEMP_PREFIX}{}", new_id()));
    fs::write(&orphan, "left behind").unwrap();
    // 还有待完成记录的：可能正要放。
    let id = f.files.create_upload(new_upload(5)).await.unwrap();
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

fn fail_dir_sync(dir: PathBuf, times: usize) {
    crate::plugin::store::DIR_SYNC_FAILURES
        .lock()
        .unwrap()
        .insert(dir, times);
}

/// 名字和样子没落盘就链接的话，断电后分不清名字上的文件是不是这个上传放的。
#[tokio::test]
async fn nothing_is_linked_until_the_name_and_look_are_durable() {
    let f = Fixture::new();
    let id = f.files.create_upload(new_upload(5)).await.unwrap();
    fs::write(f.uploads_dir().join(id.as_str()), "hello").unwrap();
    fail_dir_sync(f.dir.path().join("files/pending"), 1);
    assert!(f.files.finish_upload(&id).await.is_err());
    assert!(!f.shares_dir.join(TRANSFERS).join("a.txt").exists());
    assert!(f.files.finish_upload(&id).await.is_ok());
    assert!(f.shares_dir.join(TRANSFERS).join("a.txt").exists());
}

/// 主人改过的文件留着，补的记录没落盘时待完成记录不能删：断电后记录没了，文件就再也没人管。
#[tokio::test]
async fn a_kept_file_keeps_its_pending_record_until_the_record_is_durable() {
    let f = Fixture::new();
    let (id, _data, _temp) = placed_without_record(&f, "hello").await;
    let transfers = f.shares_dir.join(TRANSFERS);
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

/// 作废要动 `transfers`，存储卡住时可能很久：别的上传问进度不用等，正在作废的如实说接不上了。
#[tokio::test]
async fn a_stuck_clean_up_holds_up_no_other_upload() {
    let f = Fixture::new();
    let stale = f.files.create_upload(new_upload(5)).await.unwrap();
    let hold = f.files.uploads.abandon_stalled.write().await;
    let files = f.files.clone();
    let sweeping = tokio::spawn(async move { files.sweep(now_secs() + 2 * STALE_SECS).await });
    // 只试着拿锁：作废时要是还拿着它，这里报超时，而不是一起卡住。
    tokio::time::timeout(Duration::from_secs(5), async {
        while !f
            .files
            .uploads
            .abandoning
            .try_lock()
            .is_ok_and(|abandoning| !abandoning.is_empty())
        {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .expect("作废的时候不拿着这把锁");
    let other = new_id();
    f.files.pending_for_test(&other, 5, "phone");
    let seen = tokio::time::timeout(Duration::from_secs(5), f.files.upload_seen(&other));
    assert!(seen.await.expect("别的上传不用等"));
    assert!(!f.files.upload_seen(&stale).await, "这个已经接不上了");
    drop(hold);
    sweeping.await.unwrap();
    assert_eq!(
        f.files.upload_state(&stale).await.unwrap(),
        UploadState::Unknown
    );
    assert!(f.files.uploads.abandoning.lock().unwrap().is_empty());
}

/// tusd 没有它了，放下的文件主人改过、留下了：如实说传完了，不让客户端重传。
#[tokio::test]
async fn a_lost_upload_whose_file_was_kept_reads_as_finished() {
    let f = Fixture::new();
    let (id, _data, _temp) = placed_without_record(&f, "hello").await;
    let transfers = f.shares_dir.join(TRANSFERS);
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
    let transfers = f.shares_dir.join(TRANSFERS);
    fs::rename(transfers.join("a.txt"), transfers.join(TRICKY)).unwrap();
    let record = f.files.finish_upload(&id).await.unwrap();
    assert_eq!(record.name, "reportfdp.exe");
    let Origin::Upload { reference, .. } = &record.origin else {
        panic!("{record:?}");
    };
    assert_eq!(reference.path, [TRICKY]);
    assert!(f.files.shares.open(reference).is_ok());

    // 作废时留下的也一样。
    let f = Fixture::new();
    let (id, _data, _temp) = placed_without_record(&f, "hello").await;
    let transfers = f.shares_dir.join(TRANSFERS);
    fs::rename(transfers.join("a.txt"), transfers.join(TRICKY)).unwrap();
    abandon(&f, &id, "取消").await;
    let Lookup::Found(record) = f.files.registry.get(&id, now_secs()) else {
        panic!("主人改过名的文件要留着");
    };
    assert_eq!(record.name, "reportfdp.exe");
}

#[test]
fn a_file_system_out_of_inodes_has_no_room() {
    assert!(out_of_inodes(100, 0));
    assert!(!out_of_inodes(100, 1));
    assert!(!out_of_inodes(0, 0), "btrfs 不报 inode 数");
}
