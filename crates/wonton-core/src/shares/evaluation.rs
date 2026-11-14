use wonton_protocol::ShareRecipient;

use crate::friends::{FriendRecord, FriendStatus, Snapshot as FriendsSnapshot};
use crate::roles::Roles;

use super::types::REVOCATION_RETENTION_MS;
use super::{Access, GatewayRefusal, GrantPermit, Snapshot, StoredGrant};

pub(super) fn recipient_matches(stored: &StoredGrant, friend: &FriendRecord) -> bool {
    match &stored.grant.recipient {
        ShareRecipient::Friend { user_id } => user_id == &friend.user_id,
        _ => false,
    }
}

pub(super) fn generation_matches(stored: &StoredGrant, friend: &FriendRecord) -> bool {
    match &stored.grant.recipient {
        ShareRecipient::Friend { .. } => stored.direct_generation == Some(friend.access_generation),
        _ => false,
    }
}

/// 好友能看到哪些应用先按他的角色算（ADR-0088 第 4 节）：分享只是把角色范围里的应用临时给到个人。
/// 角色被删时按没有这个角色处理（最严），不猜。
pub(super) fn role_allows(roles: &Roles, friend: &FriendRecord, plugin_id: &str) -> bool {
    roles
        .get(&friend.role_id)
        .is_some_and(|role| role.permissions.open_apps.allows_plugin(plugin_id))
}

pub(super) fn evaluate(
    shares: &Snapshot,
    friends: &FriendsSnapshot,
    roles: &Roles,
    user_id: &str,
    plugin_id: &str,
    now_ms: u64,
) -> Result<Access, GatewayRefusal> {
    let recent = shares.revocations.iter().any(|r| {
        r.user_id == user_id
            && r.plugin_id == plugin_id
            && now_ms < r.at_ms.saturating_add(REVOCATION_RETENTION_MS)
    });
    let friend = match friends.get(user_id) {
        Some(friend) => friend,
        None => {
            return Err(
                if recent
                    || friends
                        .friends
                        .iter()
                        .any(|f| f.user_id == user_id && f.status == FriendStatus::Removed)
                {
                    GatewayRefusal::ShareRevoked
                } else {
                    GatewayRefusal::ShareRequired
                },
            );
        }
    };
    // 角色把应用挪出范围后，授权还在但对这个好友不再生效：对连接来说这就是“已停止分享”。
    let in_scope = role_allows(roles, friend, plugin_id);
    let mut permits = Vec::new();
    let mut expiry = Some(0);
    let mut expired = false;
    let mut revoked = recent;
    for stored in shares
        .grants
        .iter()
        .filter(|s| s.grant.plugin_id == plugin_id && recipient_matches(s, friend))
    {
        if !generation_matches(stored, friend) {
            revoked = true;
            continue;
        }
        if stored.grant.expires_at_ms.is_some_and(|at| now_ms >= at) {
            expired = true;
            continue;
        }
        if !in_scope {
            revoked = true;
            continue;
        }
        permits.push(GrantPermit {
            grant_id: stored.grant.id.clone(),
            generation: stored.generation,
        });
        expiry = match (expiry, stored.grant.expires_at_ms) {
            (Some(previous), Some(next)) => Some(previous.max(next)),
            _ => None,
        };
    }
    if !permits.is_empty() {
        Ok(Access {
            friend: friend.clone(),
            expires_at_ms: expiry,
            permits,
        })
    } else if expired {
        // 收回后马上又授权、新授权又到期：对这条连接来说「到期」才是当下的事实。
        Err(GatewayRefusal::ShareExpired)
    } else if revoked {
        Err(GatewayRefusal::ShareRevoked)
    } else {
        Err(GatewayRefusal::ShareRequired)
    }
}
