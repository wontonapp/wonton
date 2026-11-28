//! 访客链接的存储行为：有效期、条数上限、收回和过期、落盘。

use super::*;
use tempfile::TempDir;
use wonton_protocol::{GUEST_LINKS_MAX, GUEST_TTL_DEFAULT, GUEST_TTL_MAX};

fn store() -> (TempDir, GuestStore) {
    let dir = tempfile::tempdir().unwrap();
    let store = GuestStore::open(dir.path()).unwrap();
    (dir, store)
}

#[test]
fn token_is_128_bit_lower_base32() {
    let (_dir, store) = store();
    let _ = store.create("app.demo", "聚会", 0, None).unwrap();
    let text = std::fs::read_to_string(_dir.path().join(FILE_NAME)).unwrap();
    let file: File = serde_json::from_str(&text).unwrap();
    let token = &file.plugins["app.demo"][0].token;
    // 16 字节按 5 位一组编码成 26 个字符，小写字母表之外只出现 2–7。
    assert_eq!(token.len(), 26);
    assert!(
        token
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || matches!(byte, b'2'..=b'7'))
    );
}

#[test]
fn ttl_defaults_to_four_hours_and_caps_at_seven_days() {
    let (_dir, store) = store();
    let created = store.create("app.demo", "聚会", 0, None).unwrap();
    let span = created.expires_at_ms - now_ms();
    let expected = u64::from(GUEST_TTL_DEFAULT) * 1000;
    assert!(span <= expected && expected - span < 5000, "{span}");
    assert_eq!(
        store
            .create("app.demo", "长", GUEST_TTL_MAX + 1, None)
            .unwrap_err()
            .to_string(),
        GuestError::InvalidTtl.to_string()
    );
    let long = store.create("app.demo", "长", GUEST_TTL_MAX, None).unwrap();
    assert!(long.expires_at_ms - now_ms() > GUEST_TTL_MAX as u64 * 1000 - 5000);
}

#[test]
fn twenty_active_links_maximum() {
    let (_dir, store) = store();
    for number in 0..GUEST_LINKS_MAX {
        store
            .create("app.demo", &format!("第 {number} 条"), 0, None)
            .unwrap();
    }
    assert_eq!(
        store
            .create("app.demo", "多出来的", 0, None)
            .unwrap_err()
            .to_string(),
        GuestError::TooMany.to_string()
    );
    let first = store.list("app.demo")[0].id.clone();
    store.revoke("app.demo", &first).unwrap();
    store.create("app.demo", "腾出来的", 0, None).unwrap();
    assert_eq!(store.list("app.demo").len(), GUEST_LINKS_MAX);
}

#[test]
fn label_and_landing_path_are_checked() {
    let (_dir, store) = store();
    for label in ["", "  ", "两\n行"] {
        assert!(matches!(
            store.create("app.demo", label, 0, None),
            Err(GuestError::InvalidLabel)
        ));
    }
    for path in [
        "room",
        "//elsewhere.test/room",
        "/room?x=1",
        "/room x",
        "/\\evil.test/room",
    ] {
        assert!(matches!(
            store.create("app.demo", "聚会", 0, Some(path)),
            Err(GuestError::InvalidPath)
        ));
    }
    let ok = store
        .create("app.demo", "聚会", 0, Some("/room/abc"))
        .unwrap();
    assert_eq!(ok.path.as_deref(), Some("/room/abc"));
}

#[test]
fn created_links_survive_reopen_and_expired_ones_are_dropped() {
    let dir = tempfile::tempdir().unwrap();
    let store = GuestStore::open(dir.path()).unwrap();
    let kept = store.create("app.demo", "留着", 0, None).unwrap();
    store.create("app.old", "过期", 0, None).unwrap();
    let path = dir.path().join(FILE_NAME);
    let mut file: File = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    for record in file.plugins.get_mut("app.old").unwrap() {
        record.expires_at_ms = now_ms() - 1;
    }
    std::fs::write(&path, serde_json::to_string(&file).unwrap()).unwrap();
    drop(store);

    let reopened = GuestStore::open(dir.path()).unwrap();
    assert_eq!(reopened.list("app.demo").len(), 1);
    assert_eq!(reopened.list("app.demo")[0].id, kept.id);
    assert!(reopened.list("app.old").is_empty());
    let token = file.plugins["app.old"][0].token.clone();
    // 重开时过期的已经清掉，令牌看起来就是“不认识”。
    assert!(reopened.authenticate("app.old", &token).is_err());
}

#[test]
fn revoke_drops_the_link_and_sets_the_guard() {
    let (_dir, store) = store();
    let created = store.create("app.demo", "聚会", 0, None).unwrap();
    let file: File =
        serde_json::from_str(&std::fs::read_to_string(_dir.path().join(FILE_NAME)).unwrap())
            .unwrap();
    let token = file.plugins["app.demo"][0].token.clone();
    let mut auth = store.authenticate("app.demo", &token).unwrap();
    assert!(!*auth.revoke.borrow_and_update());
    store.revoke("app.demo", &created.id).unwrap();
    assert!(*auth.revoke.borrow_and_update());
    assert!(store.get("app.demo", &created.id).is_err());
    assert_eq!(
        store.authenticate("app.demo", &token).unwrap_err(),
        TokenError::Wrong
    );
    assert!(matches!(
        store.revoke("app.demo", &created.id),
        Err(GuestError::NotFound)
    ));
}

#[test]
fn tokens_belong_to_their_plugin_only() {
    let (_dir, store) = store();
    store.create("app.other", "别的", 0, None).unwrap();
    let file: File =
        serde_json::from_str(&std::fs::read_to_string(_dir.path().join(FILE_NAME)).unwrap())
            .unwrap();
    let token = file.plugins["app.other"][0].token.clone();
    assert_eq!(
        store.authenticate("app.demo", &token).unwrap_err(),
        TokenError::Wrong
    );
    assert_eq!(
        store.authenticate("app.other", "not-a-token").unwrap_err(),
        TokenError::Wrong
    );
}

#[test]
fn last_used_is_recorded() {
    let (_dir, store) = store();
    let created = store.create("app.demo", "聚会", 0, None).unwrap();
    assert_eq!(created.last_used_at_ms, None);
    store.note_used("app.demo", &created.id);
    let now = now_ms();
    assert!(
        store
            .get("app.demo", &created.id)
            .unwrap()
            .last_used_at_ms
            .unwrap()
            <= now
    );
}

#[test]
fn storage_file_is_private() {
    use std::os::unix::fs::PermissionsExt;
    let (dir, store) = store();
    store.create("app.demo", "聚会", 0, None).unwrap();
    let mode = std::fs::metadata(dir.path().join(FILE_NAME))
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o600);
}
