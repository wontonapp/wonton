use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use iroh::SecretKey;
use wonton_protocol::{MEMBER_PAIRING_CODE_TTL_SECS, MemberStatus};

use super::{JoinCheck, MemberEvent, MemberRecord, Members};
use crate::device::Devices;

#[test]
fn approval_survives_restart_and_expires_at_the_original_deadline() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("members.json");
    let clock = Arc::new(AtomicU64::new(1_000_000));
    let now = {
        let clock = clock.clone();
        Arc::new(move || clock.load(Ordering::SeqCst))
    };
    let members = Members::with_clock(&path, now.clone()).unwrap();
    let devices = Devices::open(dir.path().join("devices.json")).unwrap();
    let endpoint = SecretKey::generate().public();
    members.invite(MemberRecord::for_test("alice")).unwrap();
    let code = members.issue_pairing_code("alice").unwrap();
    assert_eq!(
        members
            .request_join(code.secret.expose(), endpoint, "手机", &devices)
            .unwrap(),
        JoinCheck::Pending("friend".into())
    );
    let id = members.join_requests()[0].id.clone();
    assert!(
        !std::fs::read_to_string(&path)
            .unwrap()
            .contains(code.secret.expose())
    );
    let members = Members::with_clock(&path, now.clone()).unwrap();
    assert_eq!(members.join_requests()[0].id, id);
    let mut events = members.subscribe();
    members.approve_join(&id, &devices).unwrap();
    assert_eq!(
        events.try_recv().unwrap(),
        MemberEvent::Joined {
            username: "alice".into()
        }
    );
    assert!(members.approve_join(&id, &devices).is_err());
    assert!(events.try_recv().is_err());
    let members = Members::with_clock(&path, now).unwrap();
    assert_eq!(members.get("alice").unwrap().status, MemberStatus::Joined);
    clock.store(code.expires_at_ms - 1, Ordering::SeqCst);
    assert_eq!(
        members
            .request_join(code.secret.expose(), endpoint, "重试", &devices)
            .unwrap(),
        JoinCheck::Paired
    );
    clock.store(code.expires_at_ms, Ordering::SeqCst);
    assert_eq!(
        members
            .request_join(code.secret.expose(), endpoint, "到期", &devices)
            .unwrap(),
        JoinCheck::Expired
    );
    assert_eq!(
        code.expires_at_ms,
        1_000_000 + MEMBER_PAIRING_CODE_TTL_SECS * 1000
    );
}

#[test]
fn racing_endpoints_share_one_claim_and_revocation_cannot_pair_back() {
    let dir = tempfile::tempdir().unwrap();
    let members = Arc::new(Members::open(dir.path().join("members.json")).unwrap());
    let devices = Arc::new(Devices::open(dir.path().join("devices.json")).unwrap());
    members.invite(MemberRecord::for_test("alice")).unwrap();
    let code = members.issue_pairing_code("alice").unwrap();
    let handles: Vec<_> = (0..2)
        .map(|_| {
            let (members, devices, secret) =
                (members.clone(), devices.clone(), code.secret.clone());
            std::thread::spawn(move || {
                let endpoint = SecretKey::generate().public();
                (
                    endpoint,
                    members
                        .request_join(secret.expose(), endpoint, "手机", &devices)
                        .unwrap(),
                )
            })
        })
        .collect();
    let results: Vec<_> = handles
        .into_iter()
        .map(|handle| handle.join().unwrap())
        .collect();
    assert_eq!(
        results
            .iter()
            .filter(|(_, result)| matches!(result, JoinCheck::Pending(_)))
            .count(),
        1
    );
    assert_eq!(
        results
            .iter()
            .filter(|(_, result)| *result == JoinCheck::CodeUsed)
            .count(),
        1
    );
    assert_eq!(members.join_requests().len(), 1);
    let endpoint = results
        .iter()
        .find(|(_, result)| matches!(result, JoinCheck::Pending(_)))
        .unwrap()
        .0;
    members
        .approve_join(&members.join_requests()[0].id, &devices)
        .unwrap();
    devices.remove(&endpoint).unwrap();
    assert_eq!(
        members
            .request_join(code.secret.expose(), endpoint, "重试", &devices)
            .unwrap(),
        JoinCheck::CodeUsed
    );
    assert!(!devices.contains(&endpoint));
}

#[test]
fn pending_device_left_by_an_interrupted_approval_is_revoked_on_restart() {
    let dir = tempfile::tempdir().unwrap();
    let members_path = dir.path().join("members.json");
    let devices_path = dir.path().join("devices.json");
    let members = Members::open(&members_path).unwrap();
    let devices = Devices::open(&devices_path).unwrap();
    members.invite(MemberRecord::for_test("alice")).unwrap();
    let code = members.issue_pairing_code("alice").unwrap();
    let endpoint = SecretKey::generate().public();
    members
        .request_join(code.secret.expose(), endpoint, "手机", &devices)
        .unwrap();
    devices
        .add_if(
            endpoint,
            Some("手机".into()),
            Some("alice".into()),
            |_| true,
        )
        .unwrap();
    drop(devices);
    drop(members);
    let members = Members::open(members_path).unwrap();
    let devices = Devices::open(devices_path).unwrap();
    members.recover_pending_devices(&devices).unwrap();
    assert!(!devices.contains(&endpoint));
    assert_eq!(members.get("alice").unwrap().status, MemberStatus::Invited);
    let id = members.join_requests()[0].id.clone();
    members.approve_join(&id, &devices).unwrap();
    assert!(devices.contains(&endpoint));
    assert_eq!(members.get("alice").unwrap().status, MemberStatus::Joined);
}

#[test]
fn failed_writes_never_publish_a_request_or_register_a_device() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("members.json");
    let members = Members::open(&path).unwrap();
    let devices = Devices::open(dir.path().join("devices.json")).unwrap();
    members.invite(MemberRecord::for_test("alice")).unwrap();
    let code = members.issue_pairing_code("alice").unwrap();
    let endpoint = SecretKey::generate().public();
    // 临时文件变成目录，原子写无法开始；不依赖运行测试的用户权限。
    let blocked = path.with_extension("json.tmp");
    std::fs::create_dir(&blocked).unwrap();
    assert!(
        members
            .request_join(code.secret.expose(), endpoint, "手机", &devices)
            .is_err()
    );
    assert!(members.join_requests().is_empty());
    assert!(!devices.contains(&endpoint));
    std::fs::remove_dir(&blocked).unwrap();
    members
        .request_join(code.secret.expose(), endpoint, "手机", &devices)
        .unwrap();
    let id = members.join_requests()[0].id.clone();
    std::fs::create_dir(&blocked).unwrap();
    assert!(members.approve_join(&id, &devices).is_err());
    assert!(!devices.contains(&endpoint));
    assert_eq!(members.get("alice").unwrap().status, MemberStatus::Invited);
    assert_eq!(members.join_requests()[0].id, id);
    std::fs::remove_dir(&blocked).unwrap();
    members.approve_join(&id, &devices).unwrap();
    assert!(devices.contains(&endpoint));
}

#[test]
fn rejection_is_reported_once_and_reissuing_voids_the_claim() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("members.json");
    let members = Members::open(&path).unwrap();
    let devices = Devices::open(dir.path().join("devices.json")).unwrap();
    members.invite(MemberRecord::for_test("alice")).unwrap();
    let code = members.issue_pairing_code("alice").unwrap();
    let endpoint = SecretKey::generate().public();
    members
        .request_join(code.secret.expose(), endpoint, "手机", &devices)
        .unwrap();
    members.reject_join(&members.join_requests()[0].id).unwrap();
    let members = Members::open(&path).unwrap();
    assert!(members.join_requests().is_empty());
    assert_eq!(
        members
            .request_join(code.secret.expose(), endpoint, "重试", &devices)
            .unwrap(),
        JoinCheck::Rejected
    );
    assert_eq!(
        members
            .request_join(code.secret.expose(), endpoint, "重试", &devices)
            .unwrap(),
        JoinCheck::CodeUsed
    );
    let fresh = members.issue_pairing_code("alice").unwrap();
    assert_eq!(
        members
            .request_join(code.secret.expose(), endpoint, "旧码", &devices)
            .unwrap(),
        JoinCheck::Unknown
    );
    members
        .request_join(fresh.secret.expose(), endpoint, "手机", &devices)
        .unwrap();
    let old_id = members.join_requests()[0].id.clone();
    members.void_pairing_code("alice").unwrap();
    assert!(members.join_requests().is_empty());
    assert!(members.approve_join(&old_id, &devices).is_err());
    assert!(!devices.contains(&endpoint));
}
