use std::fmt::Write as _;
use std::sync::Arc;

use axum::Extension;
use axum::extract::State;
use axum::response::{IntoResponse, Response};
use hyper::{HeaderMap, StatusCode};
use sha2::{Digest, Sha256};
use wonton_protocol::{BotRef, MemberStatus, PermissionItem};

use super::{Gateway, TunnelPeer, api, permissions};
use crate::media::LeaveError;
use crate::members::MemberRecord;
use crate::plugin::{InstallError, PluginId};
use crate::private_home;

impl Gateway {
    pub async fn sync_members(&self) -> Result<(), InstallError> {
        let _sync = self.member_sync.lock().await;
        self.sync_members_locked().await
    }

    pub(super) async fn sync_members_locked(&self) -> Result<(), InstallError> {
        match self.reconcile_members().await {
            Ok(()) => Ok(()),
            Err(error) => Err(self.restrict_private_bots(error).await),
        }
    }

    /// 同步半路失败（家庭群、资料、设备、某个人的 Bot……）时，后面的人还没轮到：按持久角色只收紧每个家人的
    /// 私人 Bot（见 [`crate::plugin::PluginManager::restrict_member_bot_home`]），不建、不放宽。返回原来的错误，
    /// 收紧失败的原因附在后面。
    async fn restrict_private_bots(&self, error: InstallError) -> InstallError {
        let langs = self.plugins.owner_languages();
        let mut error = error;
        for member in self.members.list() {
            let lifecycle = self.members.lifecycle(&member.username);
            let Some(bot) = member.private_bot.as_ref().or(lifecycle.bot.as_ref()) else {
                continue;
            };
            if let Err(failure) = self.plugins.restrict_member_bot_home(bot).await {
                tracing::error!(member = %member.username, bot = %bot.bot, error = %failure, "Private Bot restriction failed");
                error = error.with_note(&langs, &failure.to_string());
            }
        }
        error
    }

    async fn reconcile_members(&self) -> Result<(), InstallError> {
        let members = self.members.list();
        for member in members
            .iter()
            .filter(|member| member.status == MemberStatus::Removed)
        {
            for device in self.tunnel.devices.of_user(&member.username) {
                super::devices::revoke_device(self, &device.id, false)
                    .await
                    .map_err(storage)?;
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
            // “加好友”要管理员批或不能的，家外的邀请在 Tuwunel 就挡住（ADR-0088 第 9 节）；角色改了在这里跟着换。
            // 失败整次同步失败：操作方拿到 503，可重试，不静默吞掉。
            if let Some(matrix) = self.matrix.as_ref() {
                let add_friends = self
                    .roles
                    .get(&member.role_id)
                    .ok_or_else(|| {
                        InstallError::Invalid(format!("Unknown role: {}", member.role_id))
                    })?
                    .permissions
                    .add_friends;
                matrix
                    .sync_invite_filter(&member.username, add_friends)
                    .await?;
            }
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
            .map_err(storage)??;
        if !member.wants_private_bot {
            return Ok(());
        }
        let bot = match member.private_bot.as_ref().or(pending.bot.as_ref()) {
            Some(bot) => bot.clone(),
            None => {
                let plugin_id = self
                    .plugins
                    .default_bot_plugin()
                    .ok_or_else(|| {
                        InstallError::NotInstalled(
                            "No default Bot plugin: the default install has no Agent plugin"
                                .to_owned(),
                        )
                    })?
                    .to_string();
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
        // 媒体收干净了再停账号：收不回就停在这一步，主人重试时接着做。
        if let Some(media) = &self.media {
            media
                .leave(&member.user_id)
                .await
                .map_err(|error| media_leave_error(&member.user_id, &error))?;
        }
        let matrix = self.matrix.as_ref().ok_or_else(|| {
            InstallError::MatrixUnavailable("Matrix is not configured".to_owned())
        })?;
        matrix.deactivate(&member.user_id).await?;
        if removal.delete_home {
            let files = self.files.as_ref().ok_or_else(|| {
                InstallError::Storage(
                    "File service is not configured; cannot remove private file records".into(),
                )
            })?;
            files.remove_home(&member.username).await.map_err(storage)?;
        }
        self.members
            .removal_finished(&member.username)
            .map_err(storage)
    }
}

fn storage(error: impl std::fmt::Display) -> InstallError {
    InstallError::Storage(error.to_string())
}

/// 媒体库出错是核心自己的存储问题；提供方收不回凭据是那个插件的问题（没在跑或上游出错）。
fn media_leave_error(user: &str, error: &LeaveError) -> InstallError {
    let message = format!("收不回 {user} 的媒体：{error}");
    match error {
        LeaveError::Provider(_) => InstallError::UpstreamUnavailable(message),
        LeaveError::Store(_) | LeaveError::Task(_) | LeaveError::Session(_) => {
            InstallError::Storage(message)
        }
    }
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
