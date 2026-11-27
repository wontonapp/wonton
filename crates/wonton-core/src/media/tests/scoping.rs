use super::*;

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
