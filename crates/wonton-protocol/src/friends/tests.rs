use serde::{Serialize, de::DeserializeOwned};
use serde_json::json;

use super::*;
use crate::Action;

fn round_trip<T>(value: &T)
where
    T: Serialize + DeserializeOwned + PartialEq + std::fmt::Debug,
{
    assert_eq!(
        serde_json::from_value::<T>(serde_json::to_value(value).unwrap()).unwrap(),
        *value
    );
}

fn request(kind: FriendRequestKind) -> FriendRequest {
    FriendRequest {
        room_id: "!request:example.org".into(),
        user_id: "@bob:matrix.org".into(),
        display_name: None,
        kind,
        invited_at_ms: 1_793_000_000_000,
    }
}

#[test]
fn relationship_and_invitation_contracts_round_trip() {
    round_trip(&FriendEvent::default());
    let problem = FriendProblem {
        reason: FriendFailure::RateLimited,
        http_status: Some(429),
        matrix_errcode: Some("M_LIMIT_EXCEEDED".into()),
        retry_after_ms: Some(60_000),
    };
    let view = FriendsViewModel {
        state: FriendsState::Failed { problem },
        friends: vec![FriendRow {
            user_id: "@alice:example.org".into(),
            display_name: Some("Alice".into()),
            role_id: "friend".into(),
            shared_plugins: vec!["app.snake".into()],
            last_opened_share_at_ms: Some(1_793_000_000_001),
        }],
        incoming: vec![request(FriendRequestKind::Friend)],
        outgoing: vec![request(FriendRequestKind::DirectChat)],
        blocked: vec![FriendBlock {
            target: FriendBlockTarget::Server {
                server_name: "spam.example.org".into(),
            },
            scope: FriendBlockScope::Invitations,
            blocked_at_ms: 1_793_000_000_002,
        }],
        can_block_federation: true,
        public_access_off: true,
        my_link: Some("https://matrix.to/#/@alice:example.org".into()),
    };
    round_trip(&view);
    round_trip(&FriendsState::Loading);
    round_trip(&FriendsState::Ready);
    round_trip(&FriendBlockTarget::User {
        user_id: "@spam:example.org".into(),
    });
    round_trip(&FriendBlockScope::Federation);
}

#[test]
fn unknown_relationship_values_keep_the_document_readable() {
    assert_eq!(
        serde_json::from_value::<FriendRequestKind>(json!("team")).unwrap(),
        FriendRequestKind::Other
    );
    assert_eq!(
        serde_json::from_value::<FriendBlockTarget>(json!({"type":"network", "value":"ignored"}))
            .unwrap(),
        FriendBlockTarget::Other
    );
    assert_eq!(
        serde_json::from_value::<FriendBlockScope>(json!("future")).unwrap(),
        FriendBlockScope::Other
    );
    assert_eq!(
        serde_json::from_value::<FriendsState>(json!({"state":"refreshing", "progress":1}))
            .unwrap(),
        FriendsState::Other
    );
    assert_eq!(
        serde_json::from_value::<FriendFailure>(json!("new_failure")).unwrap(),
        FriendFailure::Other
    );
    let event: FriendEvent = serde_json::from_value(json!({"future_field": true})).unwrap();
    assert_eq!(event, FriendEvent::default());
}

#[test]
fn friendship_operations_preserve_targets_and_scope() {
    let target = FriendBlockTarget::Server {
        server_name: "spam.example.org".into(),
    };
    for action in [
        Action::RefreshFriends,
        Action::AddFriend {
            user_id: "@bob:matrix.org".into(),
        },
        Action::ApproveFriendRequest {
            room_id: "!incoming:example.org".into(),
        },
        Action::RejectFriendRequest {
            room_id: "!rejected:example.org".into(),
        },
        Action::CancelFriendRequest {
            room_id: "!outgoing:example.org".into(),
        },
        Action::RemoveFriend {
            user_id: "@bob:matrix.org".into(),
        },
        Action::BlockFriend {
            target: target.clone(),
            scope: FriendBlockScope::Federation,
        },
        Action::UnblockFriend {
            target,
            scope: FriendBlockScope::Invitations,
        },
    ] {
        round_trip(&action);
    }
}
