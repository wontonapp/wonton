use super::*;

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
