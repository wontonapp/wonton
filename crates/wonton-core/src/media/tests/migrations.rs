use super::*;

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
    assert_eq!(version, 5);
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
