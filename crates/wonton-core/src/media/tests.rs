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
