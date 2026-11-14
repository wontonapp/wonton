use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Barrier};

use wonton_protocol::{ShareGrantUpdate, ShareRecipient};

use crate::friends::Friends;
use crate::roles::Roles;

use super::types::{MAX_REVOCATIONS, REVOCATION_RETENTION_MS};
use super::{GatewayRefusal::*, PluginOffer, Shares, SharesError, Snapshot};

const ALICE: &str = "@alice:one.example.test";
const BOB: &str = "@bob:two.example.test";

struct Rig {
    temp: tempfile::TempDir,
    friends: Arc<Friends>,
    roles: Arc<Roles>,
    shares: Shares,
    now: Arc<AtomicU64>,
}

impl Rig {
    /// “朋友”模板初始不开放任何应用：这里的分享用例先把它开放成全部，
    /// 角色范围本身的行为在 `gateway/friend_rules_tests.rs` 里测。
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let friends = Arc::new(Friends::open(temp.path().join("core/friends.json")).unwrap());
        friends.accept(ALICE, None, 1, |_| false).unwrap();
        let roles = Arc::new(crate::roles::Roles::initial_for_test());
        let now = Arc::new(AtomicU64::new(100));
        let clock = Arc::clone(&now);
        let shares = Shares::with_clock(
            temp.path().join("core/shares.json"),
            Arc::clone(&friends),
            Arc::clone(&roles),
            Arc::new(move || clock.load(Ordering::SeqCst)),
        )
        .unwrap();
        Self {
            temp,
            friends,
            roles,
            shares,
            now,
        }
    }

    fn reopen(&self) -> Shares {
        let clock = Arc::clone(&self.now);
        Shares::with_clock(
            self.temp.path().join("core/shares.json"),
            Arc::clone(&self.friends),
            Arc::clone(&self.roles),
            Arc::new(move || clock.load(Ordering::SeqCst)),
        )
        .unwrap()
    }
}

fn offer() -> PluginOffer {
    PluginOffer {
        plugin_id: "game".into(),
        installed: true,
        has_webui: true,
        per_user_accounts: true,
    }
}

fn direct(expiry: Option<u64>) -> ShareGrantUpdate {
    ShareGrantUpdate {
        plugin_id: "game".into(),
        recipient: ShareRecipient::Friend {
            user_id: ALICE.into(),
        },
        expires_at_ms: expiry,
        acknowledge_risk: false,
    }
}

#[test]
fn direct_grants_use_the_most_permissive_union_and_exact_expiry() {
    let rig = Rig::new();
    let early = rig.shares.create(&direct(Some(200)), &offer()).unwrap();
    let late = rig.shares.create(&direct(Some(300)), &offer()).unwrap();
    assert_eq!(
        rig.shares
            .evaluate(ALICE, "game", 199)
            .unwrap()
            .expires_at_ms,
        Some(300)
    );
    assert_eq!(
        rig.shares
            .evaluate(ALICE, "game", 200)
            .unwrap()
            .expires_at_ms,
        Some(300)
    );
    assert_eq!(
        rig.shares
            .evaluate(ALICE, "game", 299)
            .unwrap()
            .expires_at_ms,
        Some(300)
    );
    assert_eq!(rig.shares.evaluate(ALICE, "game", 300), Err(ShareExpired));
    assert_eq!(rig.shares.next_deadline(100), Some(200));
    assert_eq!(rig.shares.next_deadline(200), Some(300));
    assert_eq!(rig.shares.next_deadline(300), None);
    let forever = rig.shares.create(&direct(None), &offer()).unwrap();
    assert_eq!(
        rig.shares
            .evaluate(ALICE, "game", 1000)
            .unwrap()
            .expires_at_ms,
        None
    );
    assert_eq!(
        rig.shares
            .plugin_shares(&offer(), 100)
            .unwrap()
            .friend_count,
        1
    );
    assert_eq!(rig.shares.active_plugin_ids(ALICE, 1000), vec!["game"]);
    rig.shares.revoke(&early.id).unwrap();
    rig.shares.revoke(&late.id).unwrap();
    assert!(rig.shares.evaluate(ALICE, "game", 1000).is_ok());
    rig.shares.revoke(&forever.id).unwrap();
    assert_eq!(rig.shares.evaluate(ALICE, "game", 1000), Err(ShareRevoked));
}

#[test]
fn friendship_reacceptance_never_revives_direct_grants_after_reopen() {
    let rig = Rig::new();
    let direct_grant = rig.shares.create(&direct(None), &offer()).unwrap();
    let generation = rig
        .shares
        .evaluate(ALICE, "game", 100)
        .unwrap()
        .friend
        .access_generation;
    rig.friends.remove(ALICE, 0).unwrap();
    assert_eq!(rig.shares.evaluate(ALICE, "game", 100), Err(ShareRevoked));
    rig.friends.accept(ALICE, None, 101, |_| false).unwrap();
    assert_eq!(rig.shares.evaluate(ALICE, "game", 100), Err(ShareRevoked));
    let reopened = rig.reopen();
    assert_eq!(reopened.evaluate(ALICE, "game", 100), Err(ShareRevoked));
    let refreshed = reopened
        .update(&direct_grant.id, &direct(None), &offer())
        .unwrap();
    assert_eq!(refreshed.id, direct_grant.id);
    assert_ne!(
        reopened
            .evaluate(ALICE, "game", 100)
            .unwrap()
            .friend
            .access_generation,
        generation
    );
    reopened.revoke(&direct_grant.id).unwrap();
    assert_eq!(reopened.evaluate(ALICE, "game", 100), Err(ShareRevoked));
}

#[test]
fn complete_replacement_changes_plugin_recipient_expiry_and_risk_gate() {
    let rig = Rig::new();
    rig.friends.accept(BOB, None, 2, |_| false).unwrap();
    let grant = rig.shares.create(&direct(None), &offer()).unwrap();
    let mut replacement = direct(Some(200));
    replacement.plugin_id = "photos".into();
    replacement.recipient = ShareRecipient::Friend {
        user_id: BOB.into(),
    };
    let mut photos = offer();
    photos.plugin_id = "photos".into();
    photos.per_user_accounts = false;
    let before = rig.shares.snapshot();
    assert!(matches!(
        rig.shares.update(&grant.id, &replacement, &photos),
        Err(SharesError::RiskAcknowledgementRequired)
    ));
    assert_eq!(rig.shares.snapshot(), before);
    replacement.acknowledge_risk = true;
    let changed = rig.shares.update(&grant.id, &replacement, &photos).unwrap();
    assert_eq!(changed.id, grant.id);
    assert_eq!(changed.expires_at_ms, Some(200));
    assert_eq!(rig.shares.evaluate(ALICE, "game", 100), Err(ShareRevoked));
    assert_eq!(
        rig.shares.evaluate(ALICE, "photos", 100),
        Err(ShareRequired)
    );
    assert!(rig.shares.evaluate(BOB, "photos", 100).is_ok());
    assert_eq!(rig.shares.evaluate(BOB, "photos", 200), Err(ShareExpired));
}

#[test]
fn unavailable_plugins_invalid_recipients_and_nonfuture_expiry_reject_without_mutation() {
    let rig = Rig::new();
    let before = rig.shares.snapshot();
    let mut absent = offer();
    absent.installed = false;
    assert!(rig.shares.create(&direct(None), &absent).is_err());
    let mut no_webui = offer();
    no_webui.has_webui = false;
    assert!(rig.shares.create(&direct(None), &no_webui).is_err());
    let mut mismatched = offer();
    mismatched.plugin_id = "other".into();
    assert!(rig.shares.create(&direct(None), &mismatched).is_err());
    assert!(rig.shares.create(&direct(Some(100)), &offer()).is_err());
    assert!(rig.shares.create(&direct(Some(99)), &offer()).is_err());
    let mut unknown = direct(None);
    unknown.recipient = ShareRecipient::Other;
    assert!(rig.shares.create(&unknown, &offer()).is_err());
    unknown.recipient = ShareRecipient::Friend {
        user_id: BOB.into(),
    };
    assert!(rig.shares.create(&unknown, &offer()).is_err());
    assert!(rig.shares.revoke("missing").is_err());
    assert!(
        rig.shares
            .update("missing", &direct(None), &offer())
            .is_err()
    );
    assert_eq!(rig.shares.snapshot(), before);
    assert!(rig.shares.plugin_shares(&absent, 100).is_err());
}

#[test]
fn uninstall_removes_grants_permanently_and_refusal_precedence_is_deterministic() {
    let rig = Rig::new();
    assert_eq!(rig.shares.evaluate(ALICE, "game", 100), Err(ShareRequired));
    let expired = rig.shares.create(&direct(Some(200)), &offer()).unwrap();
    assert_eq!(rig.shares.evaluate(ALICE, "game", 200), Err(ShareExpired));
    let second = rig.shares.create(&direct(None), &offer()).unwrap();
    assert!(rig.shares.evaluate(ALICE, "game", 200).is_ok());
    rig.shares.revoke(&second.id).unwrap();
    // 旧授权还挂着到期：此刻最准的类别是「到期」，不是「收回」。
    assert_eq!(rig.shares.evaluate(ALICE, "game", 200), Err(ShareExpired));
    // 到期的这条也收回之后，没有别的授权了，这类才是「收回」。
    rig.shares.revoke(&expired.id).unwrap();
    let third = rig.shares.create(&direct(Some(300)), &offer()).unwrap();
    rig.shares.revoke(&third.id).unwrap();
    assert_eq!(rig.shares.evaluate(ALICE, "game", 200), Err(ShareRevoked));
    rig.shares.revoke_plugin("game").unwrap();
    assert!(rig.shares.get(&expired.id).is_none());
    assert!(
        rig.shares
            .plugin_shares(&offer(), 100)
            .unwrap()
            .grants
            .is_empty()
    );
    let reopened = rig.reopen();
    assert_eq!(reopened.evaluate(ALICE, "game", 200), Err(ShareRevoked));
    assert!(reopened.active_plugin_ids(ALICE, 100).is_empty());
    assert_eq!(
        reopened.evaluate(ALICE, "game", 100 + REVOCATION_RETENTION_MS),
        Err(ShareRequired)
    );
    reopened.create(&direct(None), &offer()).unwrap();
    assert!(reopened.evaluate(ALICE, "game", 200).is_ok());
}

#[tokio::test]
async fn commits_publish_full_revisions_and_failed_disk_writes_publish_nothing() {
    let rig = Rig::new();
    let mut changes = rig.shares.subscribe();
    assert_eq!(changes.borrow_and_update().revision, 0);
    let grant = rig.shares.create(&direct(None), &offer()).unwrap();
    changes.changed().await.unwrap();
    assert_eq!(changes.borrow_and_update().grants[0].grant, grant);
    rig.shares
        .update(&grant.id, &direct(Some(200)), &offer())
        .unwrap();
    changes.changed().await.unwrap();
    let before = Arc::clone(&changes.borrow_and_update());
    assert_eq!(before.revision, 2);
    let path = rig.temp.path().join("core/shares.json");
    fs::rename(&path, rig.temp.path().join("saved.json")).unwrap();
    fs::create_dir(&path).unwrap();
    assert!(rig.shares.revoke(&grant.id).is_err());
    assert!(rig.shares.revoke_plugin("game").is_err());
    assert!(
        rig.shares
            .update(&grant.id, &direct(None), &offer())
            .is_err()
    );
    assert!(rig.shares.create(&direct(None), &offer()).is_err());
    assert_eq!(rig.shares.snapshot(), before);
    assert!(!changes.has_changed().unwrap());
    fs::remove_dir(&path).unwrap();
    fs::rename(rig.temp.path().join("saved.json"), &path).unwrap();
    assert_eq!(rig.reopen().snapshot(), before);
    rig.shares.revoke(&grant.id).unwrap();
    changes.changed().await.unwrap();
    assert!(changes.borrow_and_update().grants.is_empty());
}

#[test]
fn concurrent_grants_are_serialized_without_lost_updates() {
    let rig = Rig::new();
    let ready = Barrier::new(9);
    std::thread::scope(|scope| {
        for _ in 0..8 {
            let shares = &rig.shares;
            let ready = &ready;
            scope.spawn(move || {
                ready.wait();
                shares.create(&direct(None), &offer()).unwrap();
            });
        }
        ready.wait();
    });
    let snapshot = rig.shares.snapshot();
    assert_eq!(snapshot.revision, 8);
    assert_eq!(snapshot.grants.len(), 8);
    assert_eq!(rig.reopen().snapshot(), snapshot);
    assert_eq!(
        rig.shares
            .plugin_shares(&offer(), 100)
            .unwrap()
            .friend_count,
        1
    );
}

#[test]
fn revocation_diagnostics_are_bounded_and_expire_at_the_exact_retention_boundary() {
    let mut snapshot = Snapshot::default();
    for index in 0..MAX_REVOCATIONS + 10 {
        snapshot.remember(
            &format!("@person{index}:one.example.test"),
            "game",
            index as u64,
        );
    }
    assert_eq!(snapshot.revocations.len(), MAX_REVOCATIONS);
    assert_eq!(snapshot.revocations[0].at_ms, 10);
    snapshot.remember(ALICE, "game", 10000);
    snapshot.remember(ALICE, "game", 10001);
    assert_eq!(
        snapshot
            .revocations
            .iter()
            .filter(|r| r.user_id == ALICE)
            .count(),
        1
    );
    snapshot.prune(10001 + REVOCATION_RETENTION_MS);
    assert!(snapshot.revocations.is_empty());
}

#[test]
fn corrupt_storage_is_rejected_without_overwrite_and_replacement_is_private() {
    let rig = Rig::new();
    rig.shares.create(&direct(None), &offer()).unwrap();
    let path = rig.temp.path().join("core/shares.json");
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    fs::set_permissions(&path, fs::Permissions::from_mode(0o666)).unwrap();
    rig.shares.create(&direct(None), &offer()).unwrap();
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    let valid = serde_json::to_value(rig.shares.snapshot().as_ref()).unwrap();
    let mut cases = vec![serde_json::json!({})];
    let mut invalid = valid.clone();
    invalid["grants"][0]["direct_generation"] = 0.into();
    cases.push(invalid);
    let mut invalid = valid.clone();
    invalid["grants"][0]["grant"]["recipient"] =
        serde_json::json!({"type":"friend", "user_id":"bad"});
    cases.push(invalid);
    let mut invalid = valid.clone();
    invalid["grants"][0]["generation"] = 0.into();
    cases.push(invalid);
    let mut invalid = valid.clone();
    invalid["grants"][0]["generation"] = 999999.into();
    cases.push(invalid);
    let mut invalid = valid;
    let duplicate = invalid["grants"][0].clone();
    invalid["grants"].as_array_mut().unwrap().push(duplicate);
    cases.push(invalid);
    for invalid in cases {
        let bytes = serde_json::to_vec(&invalid).unwrap();
        fs::write(&path, &bytes).unwrap();
        assert!(matches!(
            Shares::open(&path, Arc::clone(&rig.friends), Arc::clone(&rig.roles)),
            Err(SharesError::Corrupt { .. })
        ));
        assert_eq!(fs::read(&path).unwrap(), bytes);
    }
}

#[test]
fn changing_only_expiry_does_not_falsely_classify_it_as_revoked() {
    let rig = Rig::new();
    let grant = rig.shares.create(&direct(None), &offer()).unwrap();
    rig.shares
        .update(&grant.id, &direct(Some(200)), &offer())
        .unwrap();
    assert!(rig.shares.snapshot().revocations.is_empty());
    assert_eq!(rig.shares.evaluate(ALICE, "game", 200), Err(ShareExpired));
}

#[tokio::test]
async fn coalesced_revoke_regrant_and_identical_update_never_reuse_permits() {
    let rig = Rig::new();
    let grant = rig.shares.create(&direct(None), &offer()).unwrap();
    let initial = rig.shares.evaluate(ALICE, "game", 100).unwrap();
    assert_eq!(initial.permits.len(), 1);
    assert_eq!(initial.permits[0].grant_id, grant.id);
    assert_eq!(
        initial.permits[0].generation,
        rig.shares.snapshot().revision
    );
    let mut changes = rig.shares.subscribe();
    changes.borrow_and_update();
    rig.shares.revoke(&grant.id).unwrap();
    let replacement = rig.shares.create(&direct(None), &offer()).unwrap();
    changes.changed().await.unwrap();
    let replaced = rig.shares.evaluate(ALICE, "game", 100).unwrap();
    assert_ne!(replacement.id, grant.id);
    assert!(
        initial
            .permits
            .iter()
            .all(|permit| !replaced.permits.contains(permit))
    );
    let generation = replaced.permits[0].generation;
    rig.shares
        .update(&replacement.id, &direct(None), &offer())
        .unwrap();
    let updated = rig.shares.evaluate(ALICE, "game", 100).unwrap();
    assert_eq!(updated.permits[0].grant_id, replacement.id);
    assert!(updated.permits[0].generation > generation);
    assert!(
        replaced
            .permits
            .iter()
            .all(|permit| !updated.permits.contains(permit))
    );
    assert_eq!(
        rig.reopen().evaluate(ALICE, "game", 100).unwrap().permits,
        updated.permits
    );
}
