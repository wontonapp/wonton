use std::fs;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::sync::{Arc, Barrier};

use super::{FriendStatus, Friends, FriendsError, Snapshot};

const ALICE: &str = "@alice:one.example.test";
const BOB: &str = "@alice:two.example.test";

fn open(temp: &tempfile::TempDir) -> Friends {
    Friends::open(temp.path().join("core/friends.json")).unwrap()
}

fn accept(store: &Friends, user_id: &str, username: &str) -> super::FriendRecord {
    store
        .accept_candidate(user_id, None, 10, |_| false, || Ok(username.to_owned()))
        .unwrap()
}

#[test]
fn checking_a_candidate_reserves_nothing_and_reuses_removed_identity_names() {
    let temp = tempfile::tempdir().unwrap();
    let store = open(&temp);
    let before = store.snapshot();
    let candidate = store.candidate_username(ALICE).unwrap();
    assert_eq!(store.snapshot(), before);
    assert!(
        matches!(store.accept_candidate(ALICE, None, 10, |_| true, || Ok(candidate.clone())),
        Err(FriendsError::UsernameTaken(name)) if name == candidate)
    );
    assert_eq!(store.snapshot(), before);
    let accepted = store
        .accept_candidate(ALICE, None, 10, |_| false, || Ok(candidate.clone()))
        .unwrap();
    assert_eq!(accepted.username, candidate);
    store.remove(ALICE, 1000).unwrap();
    drop(store);
    let reopened = open(&temp);
    assert_eq!(reopened.candidate_username(ALICE).unwrap(), candidate);
    let restored = reopened
        .accept_candidate(
            ALICE,
            None,
            20,
            |_| false,
            || panic!("a tombstone keeps its checked name"),
        )
        .unwrap();
    assert_eq!(restored.username, candidate);
    assert!(restored.access_generation > accepted.access_generation);
    assert!(reopened.candidate_username("invalid").is_err());
}

#[test]
fn identity_survives_rename_restart_removal_and_reacceptance() {
    let temp = tempfile::tempdir().unwrap();
    let store = open(&temp);
    let original = accept(&store, ALICE, "friend-alice");
    store
        .update_display_name(ALICE, Some("Real Matrix name"))
        .unwrap();
    store.record_opened(ALICE, 50).unwrap();
    drop(store);

    let store = open(&temp);
    let renamed = store.get(ALICE).unwrap();
    assert_eq!(renamed.username, original.username);
    assert_eq!(renamed.display_name.as_deref(), Some("Real Matrix name"));
    assert_eq!(renamed.last_opened_share_at_ms, Some(50));
    let removed = store.remove(ALICE, 1000).unwrap();
    assert_eq!(removed.status, FriendStatus::Removed);
    assert!(store.get(ALICE).is_none());
    assert!(store.find_username(&original.username).is_none());
    assert!(store.list_active().is_empty());
    assert_eq!(store.list_including_removed(), vec![removed]);
    drop(store);

    let store = open(&temp);
    let restored = store
        .accept_candidate(
            ALICE,
            None,
            900,
            |_| false,
            || panic!("reacceptance must not generate another username"),
        )
        .unwrap();
    assert_eq!(restored.username, original.username);
    assert_eq!(restored.accepted_order, original.accepted_order);
    assert_eq!(restored.accepted_at_ms, original.accepted_at_ms);
    assert!(restored.access_generation > original.access_generation);
    assert_eq!(restored.last_opened_share_at_ms, Some(50));
    assert_eq!(restored.display_name, None);
    assert_eq!(store.find_username(&original.username), Some(restored));
}

#[test]
fn a_new_friend_starts_with_the_default_invite_role() {
    let temp = tempfile::tempdir().unwrap();
    let store = open(&temp);
    let alice = accept(&store, ALICE, "friend-alice");
    assert_eq!(alice.role_id, crate::roles::DEFAULT_INVITE_ROLE);
    drop(store);
    let reopened = open(&temp);
    assert_eq!(reopened.get(ALICE).unwrap().role_id, alice.role_id);
}

#[test]
fn retained_names_cannot_cross_matrix_identities_even_after_reopen() {
    let temp = tempfile::tempdir().unwrap();
    let store = open(&temp);
    accept(&store, ALICE, "friend-occupied");
    store.remove(ALICE, 1000).unwrap();
    drop(store);
    let store = open(&temp);
    let before = store.snapshot();
    assert!(matches!(
        store.accept_candidate(BOB, None, 20, |_| false, || Ok("friend-occupied".into())),
        Err(FriendsError::UsernameTaken(_))
    ));
    assert_eq!(store.snapshot(), before);
    let bob = accept(&store, BOB, "friend-bob");
    assert_ne!(bob.username, before.friends[0].username);
    assert_eq!(store.list_active(), vec![bob]);
    assert!(store.taken("friend-occupied"));
}

#[test]
fn caller_collision_check_is_required_for_new_and_existing_identities() {
    let temp = tempfile::tempdir().unwrap();
    let store = open(&temp);
    for username in ["owner", "member", "bot", "matrix-existing"] {
        let before = store.snapshot();
        assert!(matches!(
            store.accept_candidate(
                ALICE,
                None,
                10,
                |name| name == username,
                || Ok(username.to_owned())
            ),
            Err(FriendsError::UsernameTaken(_))
        ));
        assert_eq!(store.snapshot(), before);
    }
    let alice = accept(&store, ALICE, "friend-alice");
    store.remove(ALICE, 1000).unwrap();
    let before = store.snapshot();
    assert!(matches!(
        store.accept(ALICE, None, 20, |name| name == alice.username),
        Err(FriendsError::UsernameTaken(_))
    ));
    assert_eq!(store.snapshot(), before);
}

#[test]
fn generated_names_are_valid_distinct_and_do_not_depend_on_display_names() {
    let temp = tempfile::tempdir().unwrap();
    let store = open(&temp);
    let alice = store
        .accept(ALICE, Some("同一个名字"), 10, |_| false)
        .unwrap();
    let bob = store
        .accept(BOB, Some("同一个名字"), 10, |_| false)
        .unwrap();
    assert_ne!(alice.username, bob.username);
    for friend in [alice, bob] {
        crate::owner::check_username(&friend.username).unwrap();
        crate::owner::check_not_reserved(&friend.username).unwrap();
        assert_eq!(store.find_username(&friend.username), Some(friend));
    }
}

#[test]
fn removal_records_when_it_happened_and_reacceptance_clears_it() {
    let temp = tempfile::tempdir().unwrap();
    let store = open(&temp);
    accept(&store, ALICE, "friend-alice");
    assert_eq!(store.get(ALICE).unwrap().removed_at_ms, None);

    let removed = store.remove(ALICE, 4321).unwrap();
    assert_eq!(removed.removed_at_ms, Some(4321));
    assert_eq!(store.list_including_removed()[0].removed_at_ms, Some(4321));

    let again = accept(&store, ALICE, "friend-alice");
    assert_eq!(again.removed_at_ms, None);
}

#[test]
fn acceptance_rejects_invalid_ids_and_reserved_or_invalid_plugin_names_without_change() {
    let temp = tempfile::tempdir().unwrap();
    let store = open(&temp);
    for user_id in ["alice", "@alice", "alice:one.example.test", "@alice:"] {
        assert!(store.accept(user_id, None, 10, |_| false).is_err());
    }
    for username in ["root", "nobody", "conduit", "invalid/name", "UpperCase"] {
        assert!(
            store
                .accept_candidate(ALICE, None, 10, |_| false, || Ok(username.into()))
                .is_err()
        );
    }
    assert_eq!(*store.snapshot(), Snapshot::default());
    // Matrix identity validation must retain ecosystem IDs rather than imposing POSIX localparts.
    let foreign = "@a.b=legacy:one.example.test:8448";
    assert_eq!(accept(&store, foreign, "friend-legacy").user_id, foreign);
}

#[tokio::test]
async fn deletion_publishes_complete_effective_membership_without_lag_loss() {
    let temp = tempfile::tempdir().unwrap();
    let store = open(&temp);
    let mut changes = store.subscribe();
    assert_eq!(changes.borrow_and_update().revision, 0);
    accept(&store, ALICE, "friend-alice");
    changes.changed().await.unwrap();
    let listed = Arc::clone(&changes.borrow_and_update());
    assert!(listed.get(ALICE).is_some());

    // Coalesce more mutations than an ordinary bounded event queue can retain.
    for index in 0..100 {
        store
            .update_display_name(ALICE, Some(&format!("Name {index}")))
            .unwrap();
    }
    changes.changed().await.unwrap();
    let renamed = Arc::clone(&changes.borrow_and_update());
    assert_eq!(
        renamed.get(ALICE).unwrap().display_name.as_deref(),
        Some("Name 99")
    );
    assert!(renamed.revision > listed.revision);
    store.remove(ALICE, 1000).unwrap();
    changes.changed().await.unwrap();
    let deleted = Arc::clone(&changes.borrow_and_update());
    assert!(deleted.get(ALICE).is_none());
    assert!(deleted.active().next().is_none());
    assert!(deleted.revision > renamed.revision);
    assert_eq!(deleted.friends[0].status, FriendStatus::Removed);
    assert!(listed.get(ALICE).is_some()); // Existing immutable snapshots are not mutated.
}

#[test]
fn subscribing_before_first_check_observes_an_intervening_deletion() {
    let temp = tempfile::tempdir().unwrap();
    let store = open(&temp);
    accept(&store, ALICE, "friend-alice");
    let mut changes = store.subscribe();
    assert!(changes.borrow_and_update().get(ALICE).is_some());
    store.remove(ALICE, 1000).unwrap();
    assert!(changes.has_changed().unwrap());
    assert!(changes.borrow_and_update().get(ALICE).is_none());
}

#[test]
fn invalid_changes_and_inactive_friend_updates_leave_everything_unchanged() {
    let temp = tempfile::tempdir().unwrap();
    let store = open(&temp);
    accept(&store, ALICE, "friend-alice");
    let mut changes = store.subscribe();
    changes.borrow_and_update();
    let before = store.snapshot();
    assert!(
        store
            .update_display_name(ALICE, Some("invalid\nname"))
            .is_err()
    );
    assert_eq!(store.snapshot(), before);
    assert!(!changes.has_changed().unwrap());
    store.remove(ALICE, 1000).unwrap();
    let removed = store.snapshot();
    assert!(store.update_display_name(ALICE, Some("New")).is_err());
    assert!(store.record_opened(ALICE, 20).is_err());
    assert!(store.set_role(ALICE, "friend").is_err());
    assert_eq!(store.snapshot(), removed);
}

#[test]
fn concurrent_accepted_friends_and_profile_updates_do_not_overwrite_one_another() {
    let temp = tempfile::tempdir().unwrap();
    let store = Arc::new(open(&temp));
    let ready = Arc::new(Barrier::new(9));
    std::thread::scope(|scope| {
        for index in 0..8 {
            let store = Arc::clone(&store);
            let ready = Arc::clone(&ready);
            scope.spawn(move || {
                ready.wait();
                let id = format!("@person{index}:one.example.test");
                accept(&store, &id, &format!("friend-{index}"));
                store
                    .update_display_name(&id, Some(&format!("Person {index}")))
                    .unwrap();
            });
        }
        ready.wait();
    });
    assert_eq!(store.snapshot().revision, 16);
    assert_eq!(store.list_active().len(), 8);
    for index in 0..8 {
        let friend = store
            .get(&format!("@person{index}:one.example.test"))
            .unwrap();
        assert_eq!(friend.username, format!("friend-{index}"));
        assert_eq!(friend.display_name, Some(format!("Person {index}")));
    }
    assert_eq!(open(&temp).snapshot(), store.snapshot());
}

#[test]
fn failed_persistence_changes_neither_memory_nor_subscriber_state() {
    let temp = tempfile::tempdir().unwrap();
    let store = open(&temp);
    accept(&store, ALICE, "friend-alice");
    let before = store.snapshot();
    let mut changes = store.subscribe();
    changes.borrow_and_update();
    let core = temp.path().join("core");
    let saved_core = temp.path().join("saved-core");
    fs::rename(&core, &saved_core).unwrap();
    fs::write(&core, b"blocks the parent directory").unwrap();
    assert!(store.remove(ALICE, 1000).is_err());
    assert!(store.update_display_name(ALICE, Some("Changed")).is_err());
    assert!(store.record_opened(ALICE, 100).is_err());
    assert!(store.accept(BOB, None, 20, |_| false).is_err());
    assert_eq!(store.snapshot(), before);
    assert!(!changes.has_changed().unwrap());
    fs::remove_file(&core).unwrap();
    fs::rename(saved_core, core).unwrap();
    assert_eq!(open(&temp).snapshot(), before);
}

#[test]
fn corrupt_json_and_invalid_records_are_rejected_without_overwriting() {
    let temp = tempfile::tempdir().unwrap();
    let store = open(&temp);
    accept(&store, ALICE, "friend-alice");
    let valid = serde_json::to_value(store.snapshot().as_ref()).unwrap();
    drop(store);
    let path = temp.path().join("core/friends.json");
    let mut cases = vec![serde_json::json!({})];
    let mut invalid = valid.clone();
    invalid["friends"][0]["user_id"] = "not a Matrix ID".into();
    cases.push(invalid);
    let mut invalid = valid.clone();
    let duplicate = invalid["friends"][0].clone();
    invalid["friends"].as_array_mut().unwrap().push(duplicate);
    cases.push(invalid);
    let mut invalid = valid.clone();
    invalid["friends"][0]["accepted_order"] = 0.into();
    cases.push(invalid);
    let mut invalid = valid.clone();
    invalid["friends"][0]["access_generation"] = 0.into();
    cases.push(invalid);
    let mut invalid = valid.clone();
    invalid["friends"][0]["access_generation"] = 999999.into();
    cases.push(invalid);
    let mut invalid = valid;
    invalid["friends"][0]["username"] = "invalid/name".into();
    cases.push(invalid);
    for invalid in cases {
        let bytes = serde_json::to_vec(&invalid).unwrap();
        fs::write(&path, &bytes).unwrap();
        assert!(matches!(
            Friends::open(&path),
            Err(FriendsError::Corrupt { .. })
        ));
        assert_eq!(fs::read(&path).unwrap(), bytes);
    }
    fs::write(&path, b"{ interrupted").unwrap();
    assert!(matches!(
        Friends::open(&path),
        Err(FriendsError::Corrupt { .. })
    ));
    assert_eq!(fs::read(path).unwrap(), b"{ interrupted");
}

#[test]
fn failed_atomic_rename_leaves_no_staged_registry_and_publishes_nothing() {
    let temp = tempfile::tempdir().unwrap();
    let store = open(&temp);
    accept(&store, ALICE, "friend-alice");
    let before = store.snapshot();
    let mut changes = store.subscribe();
    changes.borrow_and_update();
    let path = temp.path().join("core/friends.json");
    fs::rename(&path, temp.path().join("saved.json")).unwrap();
    fs::create_dir(&path).unwrap();
    assert!(store.remove(ALICE, 1000).is_err());
    assert_eq!(store.snapshot(), before);
    assert!(!changes.has_changed().unwrap());
    let entries: Vec<_> = fs::read_dir(temp.path().join("core"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(entries, vec![std::ffi::OsString::from("friends.json")]);
    fs::remove_dir(&path).unwrap();
    fs::rename(temp.path().join("saved.json"), &path).unwrap();
    assert_eq!(open(&temp).snapshot(), before);
}

#[test]
fn atomic_replacement_uses_a_private_inode_and_never_truncates_a_destination_symlink() {
    let temp = tempfile::tempdir().unwrap();
    let store = open(&temp);
    accept(&store, ALICE, "friend-alice");
    let path = temp.path().join("core/friends.json");
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    let old_bytes = fs::read(&path).unwrap();
    let old = fs::File::open(&path).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o666)).unwrap();
    store.update_display_name(ALICE, Some("Changed")).unwrap();
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    use std::io::Read;
    let mut still_old = Vec::new();
    (&old).read_to_end(&mut still_old).unwrap();
    assert_eq!(still_old, old_bytes);
    let target = temp.path().join("unrelated");
    fs::write(&target, b"not registry data").unwrap();
    fs::remove_file(&path).unwrap();
    symlink(&target, &path).unwrap();
    store
        .update_display_name(ALICE, Some("Changed again"))
        .unwrap();
    assert_eq!(fs::read(target).unwrap(), b"not registry data");
    assert!(
        !fs::symlink_metadata(&path)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(open(&temp).snapshot(), store.snapshot());
}

#[test]
fn access_time_is_monotonic_durable_and_noops_do_not_invalidate_readers() {
    let temp = tempfile::tempdir().unwrap();
    let store = open(&temp);
    accept(&store, ALICE, "friend-alice");
    assert_eq!(store.get(ALICE).unwrap().last_opened_share_at_ms, None);
    store.record_opened(ALICE, 100).unwrap();
    let mut changes = store.subscribe();
    changes.borrow_and_update();
    assert_eq!(store.get(ALICE).unwrap().last_opened_share_at_ms, Some(100));
    assert_eq!(
        store
            .record_opened(ALICE, 50)
            .unwrap()
            .last_opened_share_at_ms,
        Some(100)
    );
    store.record_opened(ALICE, 100).unwrap();
    store.record_opened(ALICE, 101).unwrap();
    store.update_display_name(ALICE, None).unwrap();
    assert_eq!(store.get(ALICE).unwrap().last_opened_share_at_ms, Some(101));
    assert!(!changes.has_changed().unwrap());
    // 到点落盘也不叫醒守着授权快照的连接：打开时间不进 access_generation。
    store.record_opened(ALICE, 100 + 5 * 60 * 1000).unwrap();
    assert!(!changes.has_changed().unwrap());
    assert_eq!(
        open(&temp).get(ALICE).unwrap().last_opened_share_at_ms,
        Some(100 + 5 * 60 * 1000)
    );
}

#[test]
fn a_hundred_opens_within_five_minutes_write_once_and_never_wake_the_watch() {
    let temp = tempfile::tempdir().unwrap();
    let store = open(&temp);
    accept(&store, ALICE, "friend-alice");
    let path = temp.path().join("core/friends.json");
    store.record_opened(ALICE, 0).unwrap();
    let mut changes = store.subscribe();
    changes.borrow_and_update();
    let before = fs::read(&path).unwrap();
    let revision = store.snapshot().revision;
    for step in 0..100 {
        store.record_opened(ALICE, 1_000 + step * 1_000).unwrap();
    }
    assert!(!changes.has_changed().unwrap());
    assert_eq!(store.snapshot().revision, revision);
    assert_eq!(fs::read(&path).unwrap(), before);
    // 内存里是最新的：界面和这次的返回值都看得到，只是还没落盘。
    assert_eq!(
        store.get(ALICE).unwrap().last_opened_share_at_ms,
        Some(100_000)
    );
    // 间隔从上次真正写盘的时刻（0）算，窗口内看到的更晚时间不会把写盘一直往后推。
    store.record_opened(ALICE, 5 * 60 * 1000 - 1).unwrap();
    assert!(!changes.has_changed().unwrap());
    assert_eq!(fs::read(&path).unwrap(), before);
    store.record_opened(ALICE, 5 * 60 * 1000).unwrap();
    assert!(!changes.has_changed().unwrap());
    assert_ne!(fs::read(&path).unwrap(), before);
    assert_eq!(
        open(&temp).get(ALICE).unwrap().last_opened_share_at_ms,
        Some(5 * 60 * 1000)
    );
}

#[tokio::test]
async fn coalesced_remove_and_reaccept_invalidates_old_grants_and_connection_generations() {
    let temp = tempfile::tempdir().unwrap();
    let store = open(&temp);
    let original = accept(&store, ALICE, "friend-alice");
    assert_eq!(original.access_generation, store.snapshot().revision);
    let duplicate = store.accept(ALICE, Some("Renamed"), 20, |_| false).unwrap();
    assert_eq!(duplicate.access_generation, original.access_generation);
    let mut changes = store.subscribe();
    let authenticated_generation = changes
        .borrow_and_update()
        .get(ALICE)
        .unwrap()
        .access_generation;
    store.remove(ALICE, 1000).unwrap();
    let restored = store.accept(ALICE, None, 30, |_| false).unwrap();
    changes.changed().await.unwrap();
    let latest = Arc::clone(&changes.borrow_and_update());
    let active = latest.get(ALICE).unwrap();
    assert_eq!(active.username, original.username);
    assert_ne!(active.access_generation, authenticated_generation);
    assert_eq!(active.access_generation, latest.revision);
    assert_eq!(restored.access_generation, active.access_generation);
    assert!(!changes.has_changed().unwrap());
    drop(store);
    let reopened = open(&temp);
    assert_eq!(
        reopened.get(ALICE).unwrap().access_generation,
        restored.access_generation
    );
    reopened.remove(ALICE, 1000).unwrap();
    let restored_again = reopened.accept(ALICE, None, 40, |_| false).unwrap();
    assert!(restored_again.access_generation > restored.access_generation);
}
