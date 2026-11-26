//! 总开关的落盘、权限、广播和重复下发。

use std::os::unix::fs::PermissionsExt;

use super::EntryAccess;

fn path(dir: &std::path::Path) -> std::path::PathBuf {
    dir.join("core/entry-access.json")
}

#[tokio::test]
async fn defaults_to_off_and_persists_with_private_permissions() {
    let dir = tempfile::tempdir().unwrap();
    let access = EntryAccess::open(path(dir.path())).unwrap();
    assert!(!access.enabled(), "默认关");

    let mut changes = access.subscribe();
    access
        .save(wonton_protocol::PublicAccess { enabled: true })
        .unwrap();
    assert!(access.enabled());
    changes.changed().await.unwrap();
    assert!(changes.borrow().enabled);

    let core = dir.path().join("core");
    assert_eq!(
        std::fs::metadata(&core).unwrap().permissions().mode() & 0o777,
        0o700
    );
    assert_eq!(
        std::fs::metadata(path(dir.path()))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );

    // 重新打开读回盘上的值。
    let reopened = EntryAccess::open(path(dir.path())).unwrap();
    assert!(reopened.enabled());
}

#[tokio::test]
async fn a_corrupt_file_is_an_error_not_the_default() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("core")).unwrap();
    std::fs::write(path(dir.path()), "{").unwrap();
    assert!(EntryAccess::open(path(dir.path())).is_err());
}

#[tokio::test]
async fn gates_start_unknown_and_can_be_recorded() {
    let dir = tempfile::tempdir().unwrap();
    let access = EntryAccess::open(path(dir.path())).unwrap();
    let gates = access.gates();
    assert_eq!(gates.connector, wonton_protocol::GateState::Unknown);
    access.record_gates(wonton_protocol::PublicSwitchState {
        connector: wonton_protocol::GateState::Done,
        crt_list: wonton_protocol::GateState::Done,
        core: wonton_protocol::GateState::Done,
    });
    assert_eq!(access.gates().connector, wonton_protocol::GateState::Done);
}

#[tokio::test]
async fn admin_without_containers_reports_failed_gates_and_skips_unchanged_values() {
    let dir = tempfile::tempdir().unwrap();
    let access = std::sync::Arc::new(EntryAccess::open(path(dir.path())).unwrap());
    let admin = super::EntryAdmin::new(access.clone(), None, dir.path().to_owned());
    let gates = admin.apply(true).await.unwrap();
    assert!(matches!(
        gates.connector,
        wonton_protocol::GateState::Failed { .. }
    ));
    assert_eq!(gates.core, wonton_protocol::GateState::Done);
    assert!(access.enabled());
    // 同一个值再下发：不再走闸门，结果还是上一次的。
    assert_eq!(admin.apply(true).await.unwrap(), gates);
}
