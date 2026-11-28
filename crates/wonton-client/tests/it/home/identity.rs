//! 设备身份和连接信息：身份跨重启保持不变，非法连接信息整份拒绝、什么都不改。

use std::str::FromStr;

use iroh::EndpointId;
use wonton_client::{Action, CoreError, HomeStatus};

use super::support::Phone;

#[tokio::test]
async fn a_device_keeps_its_identity_and_waits_for_connection_info() {
    let dir = tempfile::tempdir().unwrap();
    let first = Phone::open(dir.path()).home();
    assert_eq!(first.status, HomeStatus::NotConfigured);
    assert!(first.proxy.is_none());
    assert!(EndpointId::from_str(&first.device_id).is_ok(), "{first:?}");

    let again = Phone::open(dir.path()).home();
    assert_eq!(again.device_id, first.device_id, "设备身份要跨重启保持");
}

#[tokio::test]
async fn invalid_connection_info_is_rejected_and_changes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let before = phone.home();
    for ticket in ["", "{}", "not json"] {
        let result = phone.dispatch(Action::Home {
            action: wonton_protocol::HomeAction::ConnectHome {
                ticket: ticket.into(),
            },
        });
        assert!(
            matches!(result, Err(CoreError::InvalidInput { .. })),
            "{ticket}"
        );
    }
    assert_eq!(phone.home(), before);
}
