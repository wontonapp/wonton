use super::*;

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
fn a_database_at_another_version_is_refused() {
    // 比核心新的、旧的（有外部用户之前不迁移），连同从前的五版编号，都拒开，不替它补表。
    for version in [CURRENT_VERSION + 1, CURRENT_VERSION - 1, 5] {
        if version == 0 || version == CURRENT_VERSION {
            continue;
        }
        let dir = tempfile::tempdir().unwrap();
        drop(Store::open(dir.path()).unwrap());
        let conn = rusqlite::Connection::open(dir.path().join("media.sqlite3")).unwrap();
        conn.pragma_update(None, "user_version", version).unwrap();
        drop(conn);
        let err = Store::open(dir.path()).unwrap_err();
        assert!(
            matches!(err, MediaStoreError::UnsupportedVersion { found, .. } if found == version),
            "{version}: {err}"
        );
    }
}
