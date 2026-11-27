use std::sync::Arc;
use std::thread;

use tempfile::TempDir;

use wonton_protocol::media::view::QuotaEntry;
use wonton_protocol::media::{
    FailedItem, ItemState, MediaFailure, MediaFailureCode, MediaItemRef, MediaKind, MediaTask,
    PauseReason, PlaybackPlan, PlaybackSession, Progress, ProgressUnit, Queue, QueueMode,
    StopReason, TaskId, TaskKind, TaskState, TaskTarget, UploadItem,
};

use super::store::{DEFAULT_PROVIDER_UPLOAD_LIMIT, Ledger, MediaStoreError, SCHEMA_V1, Store};

const ALICE: &str = "@alice:one.example.test";
const BOB: &str = "@bob:two.example.test";
const PHONE: &str = "phone-a";
const IMMICH: &str = "immich";
const JELLYFIN: &str = "jellyfin";

fn store() -> (TempDir, Store) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    (dir, store)
}

fn item(id: &str, offset: u64) -> UploadItem {
    UploadItem {
        item_id: id.into(),
        path: format!("/photos/{id}.jpg"),
        size: 4096,
        checksum: Some(format!("sha256:{id}")),
        taken_at: Some(1_700_000_000_000),
        state: ItemState::Queued,
        upload_offset: offset,
    }
}

fn failure() -> MediaFailure {
    MediaFailure {
        code: MediaFailureCode::UpstreamError,
        message: "提供方 502".into(),
        retry_after_secs: Some(7),
    }
}

/// 一个各字段都不取默认值的上传任务：读回不一致时能直接看出是哪一列。
fn upload_task(id: u128, person: &str) -> MediaTask {
    MediaTask {
        id: TaskId::from_u128(id),
        kind: TaskKind::Upload,
        person: person.into(),
        device: PHONE.into(),
        target: TaskTarget::Upload(vec![item("one", 7), item("two", 0)]),
        state: TaskState::Paused(PauseReason::LowBattery),
        progress: Progress {
            done: 1,
            total: Some(2),
            unit: ProgressUnit::Items,
        },
        retry_count: 2,
        retry_of: Some(TaskId::from_u128(999)),
        next_attempt_at: Some(1_700_000_010),
        created_at: 1_700_000_000_000,
        partial_failures: vec![FailedItem {
            item_id: "two".into(),
            path: "/photos/two.jpg".into(),
            failure: failure(),
        }],
    }
}

fn media_item(id: &str) -> MediaItemRef {
    MediaItemRef {
        id: id.into(),
        kind: MediaKind::Photo,
        title: format!("照片 {id}"),
        artist: None,
        album: None,
        artwork_url: Some("https://provider.example.test/a.jpg".into()),
        duration_ms: Some(5_000),
        series: None,
        index: Some(3),
        played: true,
        starred: false,
        source: None,
    }
}

fn session(person: &str, id: u128) -> PlaybackSession {
    PlaybackSession {
        id: TaskId::from_u128(id),
        person: person.into(),
        device: PHONE.into(),
        item: media_item("track"),
        state: TaskState::Running,
        position_ms: 42_000,
        plan: Some(PlaybackPlan {
            session: "provider-session".into(),
            stream: wonton_protocol::media::ProviderStream::Direct(
                "https://provider.example.test/stream".into(),
            ),
            origin_stream: None,
            segments: vec![],
            audio: vec![],
            subtitles: vec![],
            seekable: true,
            duration_ms: 180_000,
            expires_at: 1_700_000_100_000,
        }),
        stop_reason: None,
    }
}

fn queue() -> Queue {
    Queue {
        revision: 1,
        entries: vec![media_item("a"), media_item("b")],
        cursor: Some(1),
        mode: QueueMode::Shuffle,
        sleep_at_ms: Some(120_000),
    }
}

#[test]
fn a_created_task_reads_back_with_every_field() {
    let (_dir, store) = store();
    let task = upload_task(1, ALICE);
    store.create_task(&task).unwrap();

    let back = store.task(ALICE, task.id).unwrap().expect("刚建的任务");
    assert_eq!(back, task);

    let offsets = store.item_offsets(ALICE, task.id).unwrap();
    assert_eq!(
        offsets,
        vec![("one".to_string(), 7), ("two".to_string(), 0)]
    );
}

#[test]
fn items_can_be_appended_and_saved_by_offset() {
    let (_dir, store) = store();
    let task = upload_task(1, ALICE);
    store.create_task(&task).unwrap();
    store
        .enqueue_items(ALICE, task.id, &[item("three", 0)])
        .unwrap();

    let mut saved = item("one", 4096);
    saved.state = ItemState::Running;
    assert!(store.save_item(ALICE, task.id, &saved).unwrap());

    let back = store.task(ALICE, task.id).unwrap().expect("任务还在");
    let TaskTarget::Upload(items) = &back.target else {
        panic!("上传任务的目标应该是上传");
    };
    assert_eq!(items.len(), 3);
    assert_eq!(items[0].upload_offset, 4096);
    assert_eq!(items[0].state, ItemState::Running);

    assert!(!store.save_item(BOB, task.id, &item("one", 1)).unwrap());
    assert!(!store.save_item(ALICE, task.id, &item("ghost", 1)).unwrap());
}

#[test]
fn a_created_session_reads_back_with_every_field() {
    let (_dir, store) = store();
    let session = session(ALICE, 2);
    store.create_session(&session).unwrap();
    assert_eq!(store.stored_session(session.id).unwrap().session, session);
    assert_eq!(store.sessions(Some(ALICE)).unwrap(), vec![session]);
}

#[test]
fn a_saved_queue_reads_back() {
    let (_dir, store) = store();
    let mut queue = queue();
    store.save_queue(ALICE, &queue).unwrap();
    assert_eq!(store.queue(ALICE).unwrap(), Some(queue.clone()));

    queue.bump();
    queue.cursor = None;
    store.save_queue(ALICE, &queue).unwrap();
    assert_eq!(store.queue(ALICE).unwrap(), Some(queue));
}

#[test]
fn people_do_not_see_each_others_rows() {
    let (_dir, store) = store();
    let mut task = upload_task(1, ALICE);
    let session = session(ALICE, 2);
    let queue = queue();
    store.create_task(&task).unwrap();
    store.create_session(&session).unwrap();
    store.save_queue(ALICE, &queue).unwrap();

    assert!(store.task(BOB, task.id).unwrap().is_none());
    // 会话按号读不分人，下面按人写才分。
    assert!(store.stored_session(session.id).is_ok());
    assert!(store.queue(BOB).unwrap().is_none());
    assert!(store.item_offsets(BOB, task.id).unwrap().is_empty());
    assert!(store.tasks(Some(BOB), None, None).unwrap().is_empty());
    assert!(store.sessions(Some(BOB)).unwrap().is_empty());

    assert_eq!(store.tasks(None, None, None).unwrap().len(), 1);
    assert_eq!(store.tasks(Some(ALICE), None, None).unwrap().len(), 1);
    assert_eq!(store.sessions(None).unwrap().len(), 1);
    assert_eq!(store.sessions(Some(ALICE)).unwrap().len(), 1);
    let version = store.stored_session(session.id).unwrap().version;

    // 拿别人的名字写，落不到 A 的行上。
    task.person = BOB.into();
    let expected = task.state.clone();
    task.state = TaskState::Running;
    assert!(!store.save_task(&task, &expected, task.retry_count).unwrap());
    let stored = store.task(ALICE, task.id).unwrap().unwrap();
    assert_eq!(stored.person, ALICE);
    assert_eq!(stored.state, TaskState::Paused(PauseReason::LowBattery));

    let mut session = session;
    session.person = BOB.into();
    session.position_ms = 1;
    assert!(!store.save_session_cas(&session, version).unwrap());
    let kept = store.stored_session(session.id).unwrap().session;
    assert_eq!(kept.person, ALICE);
    assert_eq!(kept.position_ms, 42_000);
}

#[test]
fn tasks_can_be_filtered_by_kind_and_state() {
    let (_dir, store) = store();
    let mut queued = upload_task(1, ALICE);
    queued.state = TaskState::Queued;
    let mut download = upload_task(2, ALICE);
    download.kind = TaskKind::Download;
    download.target = TaskTarget::Download(media_item("movie"));
    download.state = TaskState::Running;
    store.create_task(&queued).unwrap();
    store.create_task(&download).unwrap();

    let uploads = store
        .tasks(Some(ALICE), Some(&TaskKind::Upload), None)
        .unwrap();
    assert_eq!(uploads.len(), 1);
    assert_eq!(uploads[0].id, queued.id);

    let running = store
        .tasks(Some(ALICE), None, Some(&TaskState::Running))
        .unwrap();
    assert_eq!(running.len(), 1);
    assert_eq!(running[0].id, download.id);
    assert_eq!(
        store
            .tasks(
                Some(ALICE),
                Some(&TaskKind::Download),
                Some(&TaskState::Queued)
            )
            .unwrap()
            .len(),
        0
    );
}

#[test]
fn unknown_people_read_empty_without_creating_rows() {
    let (dir, store) = store();
    let ghost = "@ghost:example.test";
    let ghost_task = TaskId::from_u128(1);
    assert!(store.task(ghost, ghost_task).unwrap().is_none());
    assert!(store.tasks(Some(ghost), None, None).unwrap().is_empty());
    assert!(store.tasks(None, None, None).unwrap().is_empty());
    assert!(store.item_offsets(ghost, ghost_task).unwrap().is_empty());
    assert!(matches!(
        store.stored_session(ghost_task),
        Err(MediaStoreError::SessionNotFound { .. })
    ));
    assert!(store.sessions(None).unwrap().is_empty());
    assert!(store.queue(ghost).unwrap().is_none());

    let quota = store.quota(ghost).unwrap();
    assert_eq!(quota.provider_upload.used, 0);
    assert_eq!(
        quota.provider_upload.limit,
        Some(DEFAULT_PROVIDER_UPLOAD_LIMIT)
    );
    assert_eq!(quota.device_offline.used, 0);
    assert_eq!(quota.device_offline.limit, None);

    let conn = rusqlite::Connection::open(dir.path().join("media.sqlite3")).unwrap();
    for table in ["tasks", "task_items", "sessions", "queue", "quota"] {
        let count: i64 = conn
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(count, 0, "{table} 不该有行");
    }
}

#[test]
fn quota_stops_at_the_limit_and_releases_afterwards() {
    let (_dir, store) = store();
    let limit = store.quota(ALICE).unwrap().provider_upload.limit.unwrap();
    store
        .add_quota(ALICE, Ledger::ProviderUpload, Some(IMMICH), None, limit)
        .unwrap();

    let err = store
        .add_quota(ALICE, Ledger::ProviderUpload, Some(IMMICH), None, 1)
        .unwrap_err();
    assert!(
        matches!(err, MediaStoreError::QuotaExceeded { used, limit: hit, want: 1, .. } if used == limit && hit == limit),
        "{err}"
    );
    assert_eq!(store.quota(ALICE).unwrap().provider_upload.used, limit);

    store
        .take_quota(ALICE, Ledger::ProviderUpload, Some(IMMICH), None, limit)
        .unwrap();
    assert_eq!(store.quota(ALICE).unwrap().provider_upload.used, 0);
    store
        .add_quota(ALICE, Ledger::ProviderUpload, Some(IMMICH), None, 1)
        .unwrap();
}

#[test]
fn offline_quota_is_unlimited_and_kept_per_device() {
    let (_dir, store) = store();
    let half = u64::MAX / 4;
    store
        .add_quota(ALICE, Ledger::DeviceOffline, None, Some(PHONE), half)
        .unwrap();
    store
        .add_quota(ALICE, Ledger::DeviceOffline, None, Some(PHONE), half)
        .unwrap();
    let quota = store.quota(ALICE).unwrap();
    assert_eq!(quota.device_offline.used, half * 2);
    assert_eq!(quota.device_offline.limit, None);
    assert_eq!(store.quota(BOB).unwrap().device_offline.used, 0);

    let err = store
        .add_quota(ALICE, Ledger::DeviceOffline, None, None, 1)
        .unwrap_err();
    assert!(matches!(err, MediaStoreError::MissingDevice), "{err}");

    store
        .take_quota(ALICE, Ledger::DeviceOffline, None, Some(PHONE), u64::MAX)
        .unwrap();
    assert_eq!(store.quota(ALICE).unwrap().device_offline.used, 0);
}

#[test]
fn a_quota_row_keeps_its_limit_across_adds() {
    let (_dir, store) = store();
    store
        .add_quota(ALICE, Ledger::ProviderUpload, Some(IMMICH), None, 1)
        .unwrap();
    assert_eq!(
        store.quota(ALICE).unwrap().provider_upload.limit,
        Some(DEFAULT_PROVIDER_UPLOAD_LIMIT)
    );
}

#[test]
fn reopening_the_directory_keeps_everything() {
    let dir = tempfile::tempdir().unwrap();
    let task = upload_task(1, ALICE);
    let session = session(ALICE, 2);
    let queue = queue();
    {
        let store = Store::open(dir.path()).unwrap();
        store.create_task(&task).unwrap();
        store.create_session(&session).unwrap();
        store.save_queue(ALICE, &queue).unwrap();
        store
            .add_quota(ALICE, Ledger::ProviderUpload, Some(IMMICH), None, 4096)
            .unwrap();
    }

    let store = Store::open(dir.path()).unwrap();
    assert_eq!(store.task(ALICE, task.id).unwrap(), Some(task));
    assert_eq!(store.stored_session(session.id).unwrap().session, session);
    assert_eq!(store.queue(ALICE).unwrap(), Some(queue));
    assert_eq!(store.quota(ALICE).unwrap().provider_upload.used, 4096);
}

#[test]
fn a_file_that_is_not_a_database_reports_its_path() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("media.sqlite3");
    std::fs::write(&path, "这不是数据库").unwrap();

    let err = Store::open(dir.path()).unwrap_err();
    assert!(matches!(err, MediaStoreError::Corrupt { .. }), "{err}");
    assert!(
        err.to_string().contains(&path.display().to_string()),
        "{err}"
    );
}

#[test]
fn two_threads_write_without_locking_errors() {
    let (_dir, store) = store();
    let store = Arc::new(store);
    let threads: Vec<_> = [(ALICE, 0u128), (BOB, 1000)]
        .into_iter()
        .map(|(person, base)| {
            let store = Arc::clone(&store);
            thread::spawn(move || {
                for n in 0..100 {
                    store.create_task(&upload_task(base + n, person)).unwrap();
                }
            })
        })
        .collect();
    for thread in threads {
        thread.join().unwrap();
    }

    assert_eq!(store.tasks(None, None, None).unwrap().len(), 200);
    assert_eq!(store.tasks(Some(ALICE), None, None).unwrap().len(), 100);
    assert_eq!(store.tasks(Some(BOB), None, None).unwrap().len(), 100);
}

#[test]
fn quota_entry_matches_the_view_type() {
    let (_dir, store) = store();
    let quota = store.quota(ALICE).unwrap();
    assert_eq!(
        quota.provider_upload,
        QuotaEntry {
            used: 0,
            limit: Some(DEFAULT_PROVIDER_UPLOAD_LIMIT)
        }
    );
}

#[test]
fn save_task_leaves_items_to_save_item() {
    let (_dir, store) = store();
    let task = upload_task(1, ALICE);
    store.create_task(&task).unwrap();

    let mut changed = store.task(ALICE, task.id).unwrap().unwrap();
    let expected = changed.state.clone();
    changed.progress.done = 2;
    let TaskTarget::Upload(items) = &mut changed.target else {
        panic!("上传任务的目标应该是上传");
    };
    items[0].upload_offset = 4096;
    items[0].state = ItemState::Running;
    assert!(
        store
            .save_task(&changed, &expected, changed.retry_count)
            .unwrap()
    );

    let stored = store.task(ALICE, task.id).unwrap().unwrap();
    assert_eq!(stored.progress.done, 2);
    let TaskTarget::Upload(items) = stored.target else {
        panic!("上传任务的目标应该是上传");
    };
    assert_eq!(items[0].upload_offset, 7);
    assert_eq!(items[0].state, ItemState::Queued);

    let mut offset_only = items[0].clone();
    offset_only.upload_offset = 4096;
    store.save_item(ALICE, task.id, &offset_only).unwrap();
    assert_eq!(store.item_offsets(ALICE, task.id).unwrap()[0].1, 4096);
}

#[test]
fn save_task_only_lands_on_the_state_and_count_that_were_read() {
    let (_dir, store) = store();
    let task = upload_task(1, ALICE);
    store.create_task(&task).unwrap();

    // 别处先取消
    let mut cancelled = task.clone();
    cancelled.state = TaskState::Cancelled;
    assert!(
        store
            .save_task(&cancelled, &task.state, task.retry_count)
            .unwrap()
    );

    // 后到的等待网络拿旧状态写，落不上，取消不被覆盖
    let mut stale = task.clone();
    stale.state = TaskState::WaitingNetwork;
    assert!(
        !store
            .save_task(
                &stale,
                &TaskState::Paused(PauseReason::LowBattery),
                task.retry_count
            )
            .unwrap()
    );
    assert_eq!(
        store.task(ALICE, stale.id).unwrap().unwrap().state,
        TaskState::Cancelled
    );
}

#[test]
fn save_task_only_lands_on_a_retry_count_that_was_read() {
    let (_dir, store) = store();
    let task = upload_task(1, ALICE);
    store.create_task(&task).unwrap();

    let read = task.retry_count;

    // 一次失败先加了计数
    let mut first = task.clone();
    first.retry_count += 1;
    assert!(store.save_task(&first, &first.state, read).unwrap());

    // 另一次还拿老计数写，状态一样也落不上
    let mut second = task;
    second.retry_count += 1;
    assert!(!store.save_task(&second, &second.state, read).unwrap());
    assert_eq!(
        store.task(ALICE, second.id).unwrap().unwrap().retry_count,
        3
    );
}

#[test]
fn enqueueing_on_a_missing_or_download_task_errors() {
    let (_dir, store) = store();
    let missing = TaskId::from_u128(9);
    let err = store
        .enqueue_items(ALICE, missing, &[item("x", 0)])
        .unwrap_err();
    assert!(matches!(err, MediaStoreError::TaskNotFound { .. }), "{err}");

    let mut download = upload_task(1, ALICE);
    download.kind = TaskKind::Download;
    download.target = TaskTarget::Download(media_item("movie"));
    store.create_task(&download).unwrap();
    let err = store
        .enqueue_items(ALICE, download.id, &[item("x", 0)])
        .unwrap_err();
    assert!(
        matches!(err, MediaStoreError::TaskNotUpload { .. }),
        "{err}"
    );
}

#[test]
fn upload_quota_is_kept_per_provider() {
    let (_dir, store) = store();
    let limit = store.quota(ALICE).unwrap().provider_upload.limit.unwrap();
    store
        .add_quota(ALICE, Ledger::ProviderUpload, Some(IMMICH), None, limit)
        .unwrap();
    // 另一个提供方不受第一家用满的影响
    store
        .add_quota(ALICE, Ledger::ProviderUpload, Some(JELLYFIN), None, limit)
        .unwrap();
    let quota = store.quota(ALICE).unwrap();
    assert_eq!(quota.provider_upload.used, limit * 2);
    assert_eq!(quota.provider_upload.limit, Some(limit * 2));

    let err = store
        .add_quota(ALICE, Ledger::ProviderUpload, Some(IMMICH), None, 1)
        .unwrap_err();
    assert!(
        matches!(err, MediaStoreError::QuotaExceeded { .. }),
        "{err}"
    );
    assert_eq!(store.quota(BOB).unwrap().provider_upload.used, 0);
}

#[test]
fn a_migrated_v1_upload_row_blocks_its_provider_and_is_taken_over_once() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("media.sqlite3");
    let conn = rusqlite::Connection::open(&path).unwrap();
    conn.execute_batch(SCHEMA_V1).unwrap();
    for (device, used, cap) in [("", 4_096, Some(100_000)), (PHONE, 1_000, Some(10_000))] {
        conn.execute(
            "INSERT INTO quota (person, ledger, device, used, cap) VALUES (?, ?, ?, ?, ?)",
            rusqlite::params![ALICE, "provider_upload", device, used, cap],
        )
        .unwrap();
    }
    conn.execute(
        "INSERT INTO quota (person, ledger, device, used, cap) VALUES (?, ?, ?, ?, ?)",
        rusqlite::params![ALICE, "device_offline", PHONE, 2_048, Option::<i64>::None],
    )
    .unwrap();
    conn.pragma_update(None, "user_version", 1).unwrap();
    drop(conn);

    let store = Store::open(dir.path()).unwrap();
    let quota = store.quota(ALICE).unwrap();
    // v1 的上传账并成一份旧账，离线的账按设备原样留着
    assert_eq!(
        quota.provider_upload,
        QuotaEntry {
            used: 5_096,
            limit: Some(110_000),
        }
    );
    assert_eq!(quota.device_offline.used, 2_048);

    // 旧用量和旧上限都算在这个提供方头上：再记 104 905 字节就超
    let err = store
        .add_quota(ALICE, Ledger::ProviderUpload, Some(IMMICH), None, 104_905)
        .unwrap_err();
    assert!(
        matches!(
            err,
            MediaStoreError::QuotaExceeded {
                used: 5_096,
                limit: 110_000,
                want: 104_905,
                ..
            }
        ),
        "{err}"
    );
    assert_eq!(store.quota(ALICE).unwrap().provider_upload.used, 5_096);

    // 记进来以后旧账由这一家接手：用量接着算，上限不另加一份默认值
    store
        .add_quota(ALICE, Ledger::ProviderUpload, Some(IMMICH), None, 100)
        .unwrap();
    assert_eq!(
        store.quota(ALICE).unwrap().provider_upload,
        QuotaEntry {
            used: 5_196,
            limit: Some(110_000),
        }
    );

    // 旧账只被接手一次，另一家从自己的默认上限起算
    store
        .add_quota(ALICE, Ledger::ProviderUpload, Some(JELLYFIN), None, 1)
        .unwrap();
    assert_eq!(
        store.quota(ALICE).unwrap().provider_upload,
        QuotaEntry {
            used: 5_197,
            limit: Some(110_000 + DEFAULT_PROVIDER_UPLOAD_LIMIT),
        }
    );
}

#[test]
fn releasing_quota_on_a_provider_without_a_row_leaves_the_limit_alone() {
    let (dir, store) = store();
    store
        .add_quota(ALICE, Ledger::ProviderUpload, Some(IMMICH), None, 1)
        .unwrap();
    let before = store.quota(ALICE).unwrap();

    let after = store
        .take_quota(ALICE, Ledger::ProviderUpload, Some(JELLYFIN), None, 10)
        .unwrap();
    assert_eq!(after, before);
    assert_eq!(store.quota(ALICE).unwrap(), before);
    // 离线账同理，没记过的设备也不建行
    let after = store
        .take_quota(ALICE, Ledger::DeviceOffline, None, Some(PHONE), 10)
        .unwrap();
    assert_eq!(
        after.device_offline,
        QuotaEntry {
            used: 0,
            limit: None,
        }
    );

    let conn = rusqlite::Connection::open(dir.path().join("media.sqlite3")).unwrap();
    let rows: Vec<(String, String)> = conn
        .prepare("SELECT ledger, device FROM quota")
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    assert_eq!(rows, vec![("provider_upload".to_string(), String::new())]);
}

#[test]
fn upload_quota_needs_a_provider() {
    let (_dir, store) = store();
    let err = store
        .add_quota(ALICE, Ledger::ProviderUpload, None, None, 1)
        .unwrap_err();
    assert!(matches!(err, MediaStoreError::MissingProvider), "{err}");
}

#[test]
fn a_version_1_database_is_migrated_and_still_reads_and_writes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("media.sqlite3");
    let conn = rusqlite::Connection::open(&path).unwrap();
    conn.execute_batch(SCHEMA_V1).unwrap();
    conn.execute(
        "INSERT INTO quota (person, ledger, device, used, cap) VALUES (?, ?, ?, ?, ?)",
        rusqlite::params![ALICE, "provider_upload", "", 4096, 100_000],
    )
    .unwrap();
    let running = upload_task(1, ALICE);
    conn.execute(
        "INSERT INTO tasks (id, person, device, kind, target, state, state_tag, progress,
                            retry_count, retry_of, created_at, partial_failures)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        rusqlite::params![
            running.id.to_string(),
            ALICE,
            PHONE,
            "upload",
            "{\"kind\":\"upload\",\"value\":[]}",
            "\"running\"",
            "running",
            "{\"done\":0,\"total\":null,\"unit\":\"bytes\"}",
            0,
            Option::<String>::None,
            running.created_at,
            "[]",
        ],
    )
    .unwrap();
    conn.pragma_update(None, "user_version", 1).unwrap();
    drop(conn);

    let store = Store::open(dir.path()).unwrap();
    let quota = store.quota(ALICE).unwrap();
    assert_eq!(quota.provider_upload.used, 4096);
    assert_eq!(quota.provider_upload.limit, Some(100_000));

    // 旧行还在、新列可读可写
    let mut task = store.task(ALICE, running.id).unwrap().unwrap();
    assert_eq!(task.state, TaskState::Running);
    assert_eq!(task.next_attempt_at, None);
    task.next_attempt_at = Some(42);
    assert!(store.save_task(&task, &TaskState::Running, 0).unwrap());
    assert_eq!(
        store
            .task(ALICE, running.id)
            .unwrap()
            .unwrap()
            .next_attempt_at,
        Some(42)
    );

    let conn = rusqlite::Connection::open(&path).unwrap();
    let version: i64 = conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .unwrap();
    assert_eq!(version, 4);
}

#[test]
fn a_newer_database_version_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    drop(Store::open(dir.path()).unwrap());
    let conn = rusqlite::Connection::open(dir.path().join("media.sqlite3")).unwrap();
    conn.pragma_update(None, "user_version", 99).unwrap();
    drop(conn);

    let err = Store::open(dir.path()).unwrap_err();
    assert!(matches!(err, MediaStoreError::TooNew { .. }), "{err}");
}

#[test]
fn only_one_session_per_person_can_be_active() {
    let (_dir, store) = store();
    let first = session(ALICE, 1);
    store.create_session(&first).unwrap();

    let err = store.create_session(&session(ALICE, 2)).unwrap_err();
    assert!(
        matches!(err, MediaStoreError::ActiveSessionExists { .. }),
        "{err}"
    );
    assert_eq!(
        store.active_session(ALICE).unwrap().unwrap().session.id,
        first.id
    );

    let tracked = store.stored_session(first.id).unwrap();
    let mut stopped = first.clone();
    stopped.state = TaskState::Cancelled;
    stopped.stop_reason = Some(StopReason::TakenOver);
    assert!(store.save_session_cas(&stopped, tracked.version).unwrap());
    assert!(store.active_session(ALICE).unwrap().is_none());

    store.create_session(&session(ALICE, 2)).unwrap();
    assert_eq!(
        store.active_session(ALICE).unwrap().unwrap().session.id,
        TaskId::from_u128(2)
    );
    let old = store.stored_session(first.id).unwrap().session;
    assert_eq!(old.stop_reason, Some(StopReason::TakenOver));
    assert_eq!(old.position_ms, 42_000);
}

#[test]
fn a_session_write_needs_the_version_that_was_read() {
    let (_dir, store) = store();
    let session = session(ALICE, 1);
    store.create_session(&session).unwrap();
    let mut stale = store.stored_session(session.id).unwrap();

    let mut first = stale.session.clone();
    first.position_ms = 1;
    assert!(store.save_session_cas(&first, stale.version).unwrap());

    stale.session.position_ms = 2;
    assert!(
        !store
            .save_session_cas(&stale.session, stale.version)
            .unwrap()
    );
    assert_eq!(
        store
            .stored_session(session.id)
            .unwrap()
            .session
            .position_ms,
        1
    );
    assert!(!store.save_session_cas(&stale.session, 99).unwrap());
}

#[test]
fn a_missing_session_reports_its_id() {
    let (_dir, store) = store();
    let ghost = TaskId::from_u128(7);
    let err = store.stored_session(ghost).unwrap_err();
    assert!(
        matches!(err, MediaStoreError::SessionNotFound { session, .. } if session == ghost),
        "{err}"
    );
}

#[test]
fn a_version_1_database_is_migrated_to_session_versions_and_stop_reasons() {
    let dir = tempfile::tempdir().unwrap();
    let conn = rusqlite::Connection::open(dir.path().join("media.sqlite3")).unwrap();
    conn.execute_batch(SCHEMA_V1).unwrap();
    let insert = |id: u128, person: &str| {
        conn.execute(
            "INSERT INTO sessions (id, person, device, item, state, state_tag, position_ms, plan)
             VALUES (?, ?, ?, ?, ?, ?, ?, NULL)",
            rusqlite::params![
                TaskId::from_u128(id).to_string(),
                person,
                PHONE,
                serde_json::to_string(&media_item("track")).unwrap(),
                serde_json::to_string(&TaskState::Running).unwrap(),
                "running",
                42_000,
            ],
        )
        .unwrap();
    };
    insert(1, ALICE);
    insert(2, ALICE);
    insert(3, ALICE);
    insert(4, BOB);
    conn.pragma_update(None, "user_version", 1).unwrap();
    drop(conn);

    let store = Store::open(dir.path()).unwrap();
    // 每人只留最新一条活动，旧的两条落接管终态。
    for old in [1, 2] {
        let stopped = store.stored_session(TaskId::from_u128(old)).unwrap();
        assert_eq!(stopped.session.stop_reason, Some(StopReason::TakenOver));
        assert_eq!(stopped.session.state, TaskState::Cancelled);
        assert_eq!(stopped.version, 0);
    }
    assert_eq!(
        store.active_session(ALICE).unwrap().unwrap().session.id,
        TaskId::from_u128(3)
    );
    assert_eq!(
        store.active_session(BOB).unwrap().unwrap().session.id,
        TaskId::from_u128(4)
    );
    assert!(matches!(
        store.create_session(&session(ALICE, 5)),
        Err(MediaStoreError::ActiveSessionExists { .. })
    ));
}
