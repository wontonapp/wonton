use std::fmt::Write as _;
use std::sync::Arc;

use axum::Extension;
use axum::extract::State;
use axum::response::{IntoResponse, Response};
use hyper::{HeaderMap, StatusCode};
use sha2::{Digest, Sha256};
use wonton_protocol::{BotRef, MemberStatus, PermissionItem};

use super::{Gateway, TunnelPeer, api, permissions};
use crate::members::MemberRecord;
use crate::plugin::{InstallError, PluginId};
use crate::private_home;

impl Gateway {
    pub async fn sync_members(&self) -> Result<(), InstallError> {
        let _sync = self.member_sync.lock().await;
        self.sync_members_locked().await
    }

    pub(super) async fn sync_members_locked(&self) -> Result<(), InstallError> {
        let members = self.members.list();
        for member in members
            .iter()
            .filter(|member| member.status == MemberStatus::Removed)
        {
            for device in self.tunnel.devices.of_user(&member.username) {
                super::devices::revoke_device(self, &device.id).map_err(storage)?;
            }
        }
        self.sync_family_room().await?;
        for member in members
            .iter()
            .filter(|member| member.status == MemberStatus::Removed)
        {
            self.finish_member_removal(member).await?;
        }
        for member in members
            .iter()
            .filter(|member| member.status == MemberStatus::Joined)
        {
            self.provision_member(member).await?;
        }
        Ok(())
    }

    async fn provision_member(&self, member: &MemberRecord) -> Result<(), InstallError> {
        let role = self
            .roles
            .get(&member.role_id)
            .ok_or_else(|| InstallError::Invalid(format!("Unknown role: {}", member.role_id)))?;
        let pending = self.members.lifecycle(&member.username);
        if !role.permissions.private_bot {
            if let Some(bot) = member.private_bot.as_ref().or(pending.bot.as_ref()) {
                self.plugins.sync_member_bot_home(bot).await?;
            }
            return Ok(());
        }
        let data_dir = self.plugins.data_dir();
        let username = member.username.clone();
        let owner = self.plugins.private_home_owner();
        tokio::task::spawn_blocking(move || private_home::ensure(&data_dir, &username, owner))
            .await
            .map_err(storage)?
            .map_err(InstallError::Storage)?;
        if !member.wants_private_bot {
            return Ok(());
        }
        let bot = match member.private_bot.as_ref().or(pending.bot.as_ref()) {
            Some(bot) => bot.clone(),
            None => {
                let plugin_id = self.onboarding.primary_agent().await?
                    .ok_or_else(|| InstallError::NotInstalled("No primary Agent is configured; choose an Agent before retrying member setup".to_owned()))?;
                let digest = Sha256::digest(member.username.as_bytes());
                let mut name = String::with_capacity(29);
                name.push_str("user-");
                for byte in &digest[..12] {
                    write!(name, "{byte:02x}").expect("writing to a String cannot fail");
                }
                let bot = BotRef {
                    plugin_id,
                    bot: name,
                };
                self.members
                    .reserve_private_bot(&member.username, &bot)
                    .map_err(storage)?;
                bot
            }
        };
        let plugin = PluginId::parse(&bot.plugin_id)
            .map_err(|error| InstallError::Invalid(error.to_string()))?;
        self.plugins
            .ensure_member_bot(&plugin, &bot.bot, &member.username)
            .await?;
        self.plugins.sync_member_bot_home(&bot).await?;
        if member.private_bot.as_ref() != Some(&bot) {
            self.members
                .private_bot_ready(&member.username, &bot)
                .map_err(storage)?;
        }
        Ok(())
    }

    async fn finish_member_removal(&self, member: &MemberRecord) -> Result<(), InstallError> {
        let lifecycle = self.members.lifecycle(&member.username);
        let Some(removal) = lifecycle.removal else {
            return Ok(());
        };
        if let Some(bot) = member.private_bot.as_ref().or(lifecycle.bot.as_ref()) {
            self.plugins
                .remove_member_bot(bot, &member.username, removal.keep_bot_data)
                .await?;
        }
        let matrix = self.matrix.as_ref().ok_or_else(|| {
            InstallError::MatrixUnavailable("Matrix is not configured".to_owned())
        })?;
        matrix.deactivate(&member.user_id).await?;
        if removal.delete_home {
            let data_dir = self.plugins.data_dir();
            let username = member.username.clone();
            tokio::task::spawn_blocking(move || private_home::remove(&data_dir, &username))
                .await
                .map_err(storage)?
                .map_err(InstallError::Storage)?;
        }
        self.members
            .removal_finished(&member.username)
            .map_err(storage)
    }
}

fn storage(error: impl std::fmt::Display) -> InstallError {
    InstallError::Storage(error.to_string())
}

pub(super) async fn synchronize_locked(gateway: &Gateway, headers: &HeaderMap) -> Option<Response> {
    gateway
        .sync_members_locked()
        .await
        .err()
        .map(|error| sync_error(headers, &error))
}

fn sync_error(headers: &HeaderMap, error: &InstallError) -> Response {
    tracing::error!(%error, retry_path = wonton_protocol::MEMBER_SYNC_PATH, "Member setup synchronization failed");
    let message = crate::l10n::text(
        &api::languages(headers),
        "family-private-sync-failed",
        &[("error", error.to_string().into())],
    );
    api::error_response(StatusCode::SERVICE_UNAVAILABLE, error.kind(), &message)
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
    match gateway.sync_members().await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => sync_error(&headers, &error),
    }
}

#[cfg(test)]
#[path = "family_private_tests.rs"]
mod tests;
