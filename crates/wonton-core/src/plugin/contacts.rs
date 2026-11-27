//! Bot 的联系人（ADR-0045）在插件这边的两件事：注入 `WONTON_MATRIX_ALLOWED_USERS` 时把已成立的联系人加在归属人后面，
//! 名单变了重建这个 Bot 的服务。联系人记录在 [`crate::contacts`]，这里只按 Bot 的用户 ID 问它。

use std::sync::Arc;

use super::bots::is_bot;
use super::health_gate::Gate;
use super::manager::{InstallError, PluginManager, PluginStatus};
use super::manifest::PluginId;
use super::render;
use super::store::StoreError;
use super::upgrade::installed_package;
use wonton_protocol::{BotKeeper, BotRef, ContactApply, ContactApplyFailure};

/// Bot 的 Matrix 用户 ID → 它已成立的联系人。
pub type ContactSource = Arc<dyn Fn(&str) -> Vec<String> + Send + Sync>;

/// 本机的一个 Bot 的记录：插件和 Bot 名、Matrix 用户 ID、归属。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BotListing {
    pub bot: BotRef,
    pub user_id: String,
    pub keeper: BotKeeper,
}

/// 核心记录中的身份和主房间；不携带设备、令牌或部署信息。
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct BotIdentityRecordFacts {
    pub(crate) listing: BotListing,
    pub(crate) home_room: String,
}

impl PluginManager {
    /// 只读指定 Bot；无可定位的记录返回 None，真实存储错误不转成“没有”。
    pub(crate) fn bot_identity_record(
        &self,
        bot: &BotRef,
    ) -> Result<Option<BotIdentityRecordFacts>, StoreError> {
        let Ok(id) = PluginId::parse(&bot.plugin_id) else {
            return Ok(None);
        };
        Ok(self
            .store
            .bots()
            .entry(&id, &bot.bot)?
            .map(|entry| BotIdentityRecordFacts {
                listing: BotListing {
                    bot: bot.clone(),
                    user_id: entry.account.user_id,
                    keeper: entry.keeper,
                },
                home_room: entry.account.home_room,
            }))
    }

    /// 接上联系人记录。只接一次：核心启动时接，之后渲染 Bot 的变量都带上联系人。
    pub fn set_contact_source(&self, source: ContactSource) {
        if self.contact_source.set(source).is_err() {
            tracing::warn!("联系人记录已经接过了");
        }
    }

    /// 注入的名单：管它的人在前，已成立的联系人跟上，逗号分隔。
    pub(super) fn allowed_users(&self, keeper_user_id: String, bot_user_id: &str) -> String {
        let Some(source) = self.contact_source.get() else {
            return keeper_user_id;
        };
        let mut users = vec![keeper_user_id];
        for contact in source(bot_user_id) {
            if !users.contains(&contact) {
                users.push(contact);
            }
        }
        users.join(",")
    }

    /// 所有插件记录在案的 Bot。
    pub fn bot_listings(&self) -> Result<Vec<BotListing>, String> {
        let all = self.store.bots().all().map_err(|e| e.to_string())?;
        Ok(all
            .into_iter()
            .flat_map(|(id, bots)| {
                bots.bots.into_iter().map(move |(bot, entry)| BotListing {
                    bot: BotRef {
                        plugin_id: id.to_string(),
                        bot,
                    },
                    user_id: entry.account.user_id,
                    keeper: entry.keeper,
                })
            })
            .collect())
    }

    /// 管这个 Bot 的人的用户名：私人 Bot 是归属人，共享 Bot 是主人。
    pub fn keeper_username(&self, keeper: &BotKeeper) -> String {
        self.keeper_user(keeper)
    }

    /// 一个 Bot 的显示名：清单里写的，没写时用插件标题，按主人的语言挑。
    pub fn bot_display_name(&self, bot: &BotRef) -> Option<String> {
        let id = PluginId::parse(&bot.plugin_id).ok()?;
        let name = self.bot_name(&id, &bot.bot)?;
        Some(self.owner_languages().pick_owned(&name))
    }

    /// 联系人变了：插件在运行时重建这个 Bot 的服务（沿用设备和令牌），名单跟上。
    /// 回的是运行时的真实结果（#646）：插件停着、Docker 连不上、服务起不来都如实报没换上。
    /// 记录是准的，失败的 Bot 下次启动或下一次变更照记录渲染。
    pub async fn rebuild_for_contacts(&self, bot: &BotRef) -> ContactApply {
        let id = match PluginId::parse(&bot.plugin_id) {
            Ok(id) => id,
            Err(e) => return failed(ContactApplyFailure::InvalidInput, e.to_string()),
        };
        match self.status_of(&id) {
            Some(PluginStatus::Running) => {}
            Some(status) => {
                return failed(
                    ContactApplyFailure::PluginNotReady,
                    format!("插件 {id} 没在运行（{status:?}）"),
                );
            }
            None => {
                return failed(
                    ContactApplyFailure::PluginNotReady,
                    format!("插件 {id} 没有登记"),
                );
            }
        }
        match self.rebuild_running(&id, bot).await {
            Ok(()) => ContactApply::Applied,
            Err(e) => failed(failure_of(&e), e.to_string()),
        }
    }

    async fn rebuild_running(&self, id: &PluginId, bot: &BotRef) -> Result<(), InstallError> {
        let record = self
            .store
            .load(id)?
            .ok_or_else(|| InstallError::NotInstalled(id.to_string()))?;
        let langs = self.owner_languages();
        let package = installed_package(&record, &langs)?;
        if !is_bot(&package.manifest) {
            return Ok(());
        }
        if package.manifest.bots.is_some() {
            return self.rebuild_bot(id, &bot.bot).await;
        }
        // 只有一个 Bot 的插件：Bot 的身份在插件级变量里，整个插件按新变量重建。
        let lan = self.lan_grant(&package, record.lan_webui, &langs).await?;
        let mut ctx = self.rebuild_context(&package, &record, lan, None).await?;
        self.fill_bot_vars(&package, &mut ctx, &langs).await?;
        let plan = render::render(&package, &ctx, &langs)?;
        self.replace_containers(&ctx, plan, Gate::StartPeriod)
            .await?;
        self.reconcile_running(&package, &record).await.map(|_| ())
    }
}

fn failed(reason: ContactApplyFailure, detail: String) -> ContactApply {
    ContactApply::Failed { reason, detail }
}

/// 重建失败的原因归类；按 [`InstallError::kind`] 分，不认识的原样算上游出错。
fn failure_of(error: &InstallError) -> ContactApplyFailure {
    match error.kind() {
        "invalid_input" | "settings_invalid" | "settings_schema_invalid" => {
            ContactApplyFailure::InvalidInput
        }
        "permission_denied" => ContactApplyFailure::PermissionDenied,
        "docker_unavailable" | "docker_rejected" | "docker_timed_out" => {
            ContactApplyFailure::TransportUnavailable
        }
        "not_installed" | "record_unusable" | "not_found" | "core_upgrading" => {
            ContactApplyFailure::PluginNotReady
        }
        _ => ContactApplyFailure::Upstream,
    }
}
