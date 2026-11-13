use std::sync::Arc;

use axum::Extension;
use axum::extract::State;
use axum::response::{IntoResponse, Response};
use hyper::{HeaderMap, StatusCode};
use serde_json::{Map, json};
use wonton_protocol::{MemberStatus, PermissionItem, RoleTemplate};

use super::{Gateway, TunnelPeer, api, permissions};
use crate::l10n;
use crate::matrix::MatrixError;

impl Gateway {
    pub async fn sync_family_room(&self) -> Result<(), MatrixError> {
        let _sync = self.family_room_sync.lock().await;
        let members = self.members.list();
        // 未有成员加入就没有群可同步；邀请和角色草稿不需要 Matrix。
        if !members
            .iter()
            .any(|member| matches!(member.status, MemberStatus::Joined | MemberStatus::Removed))
        {
            return Ok(());
        }
        let matrix = self
            .matrix
            .as_ref()
            .ok_or_else(|| MatrixError::Unavailable {
                context: "family room".to_owned(),
                message: "Matrix is not configured".to_owned(),
            })?;
        let owner = self.owner.username();
        let mut users = Map::from_iter([(matrix.user_id(owner), json!(100))]);
        let mut family = Vec::new();
        for member in &members {
            if member.status != MemberStatus::Joined {
                continue;
            }
            let role = self
                .roles
                .get(&member.role_id)
                .ok_or_else(|| MatrixError::Invalid(format!("Unknown role: {}", member.role_id)))?;
            if matches!(
                role.template,
                Some(RoleTemplate::Friend | RoleTemplate::Other)
            ) {
                continue;
            }
            let level = if role.permissions.manage_members {
                100
            } else {
                0
            };
            users.insert(member.user_id.clone(), json!(level));
            family.push(member);
        }
        let core = matrix.core_session();
        let room = match core.family_room().await? {
            Some(room) => room,
            None if family.is_empty() => return Ok(()),
            None => {
                let name = l10n::text(&self.plugins.owner_languages(), "family-room-name", &[]);
                core.create_family_room(&name, &users).await?
            }
        };
        core.sync_family_power_levels(&room, &users).await?;
        // 从持久名单重算，失败、重启或漏掉通知后仍能补上踢人，不依赖一次性事件。
        for member in &members {
            if !users.contains_key(&member.user_id)
                && matches!(
                    core.membership(&room, &member.user_id).await?.as_deref(),
                    Some("join" | "invite")
                )
            {
                core.kick(&room, &member.user_id).await?;
            }
        }
        matrix.join_family_room(&room, owner).await?;
        for member in family {
            matrix.join_family_room(&room, &member.username).await?;
        }
        Ok(())
    }
}

pub(super) async fn synchronize(gateway: &Gateway, headers: &HeaderMap) -> Option<Response> {
    gateway.sync_family_room().await.err().map(|error| {
        tracing::error!(%error, retry_path = wonton_protocol::FAMILY_ROOM_SYNC_PATH, "Family room synchronization failed");
        let kind = match &error {
            MatrixError::Unavailable { .. } => "matrix_unavailable",
            MatrixError::Rejected { errcode, .. } if errcode == "M_FORBIDDEN" => "matrix_permission_denied",
            _ => "family_room_sync_failed",
        };
        let message = l10n::text(
            &api::languages(headers),
            "family-room-sync-failed",
            &[("error", error.to_string().into())],
        );
        api::error_response(StatusCode::SERVICE_UNAVAILABLE, kind, &message)
    })
}

pub(super) async fn retry(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
) -> Response {
    if let Some(denied) =
        permissions::require(&gateway, peer, &headers, PermissionItem::ManageMembers)
    {
        return denied;
    }
    match synchronize(&gateway, &headers).await {
        None => StatusCode::NO_CONTENT.into_response(),
        Some(response) => response,
    }
}

#[cfg(test)]
#[path = "family_room_tests.rs"]
mod tests;
