//! 待完成状态到 API `pending` 字段的映射。

use wonton_protocol::PendingOp;

use super::passwords::State;
use crate::plugin::PluginManager;

#[test]
fn pending_states_map_to_their_operation() {
    assert_eq!(
        PluginManager::pending_op(&State::PendingRevoke),
        Some(PendingOp::Revoke)
    );
    assert_eq!(
        PluginManager::pending_op(&State::PendingRotate {
            next: Some("n".into())
        }),
        Some(PendingOp::Rotate)
    );
    assert_eq!(
        PluginManager::pending_op(&State::PendingRotate { next: None }),
        Some(PendingOp::Rotate)
    );
    assert_eq!(PluginManager::pending_op(&State::Ready), None);
}
