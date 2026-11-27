//! 离线占用这本账以设备上报为准（`PUT /offline`，ADR-0113“配额与清理”）：按设备分开记，配额看这个人
//! 所有设备之和。

use super::store::{Ledger, Store};

const ALICE: &str = "@alice:one.example.test";
const BOB: &str = "@bob:two.example.test";

#[test]
fn each_device_reports_only_its_own_share() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    store.report_offline(ALICE, "phone", 300).unwrap();
    let view = store.report_offline(ALICE, "tablet", 200).unwrap();
    assert_eq!(view.device_offline.used, 500);
    // 手机删了副本再报：只改手机那份，平板的不动；别人的账不受影响。
    store.report_offline(BOB, "phone", 999).unwrap();
    let view = store.report_offline(ALICE, "phone", 100).unwrap();
    assert_eq!(view.device_offline.used, 300);
    // 两次上报之间，下载写完往上加，下一次上报再对齐。
    store
        .add_quota(ALICE, Ledger::DeviceOffline, None, Some("tablet"), 50)
        .unwrap();
    assert_eq!(store.quota(ALICE).unwrap().device_offline.used, 350);
    // 平板被移除：它那份清零。
    let view = store.report_offline(ALICE, "tablet", 0).unwrap();
    assert_eq!(view.device_offline.used, 100);
    assert_eq!(store.quota(BOB).unwrap().device_offline.used, 999);
}
