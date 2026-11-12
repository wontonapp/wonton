//! Bot 的联系人（ADR-0045）在插件这边的两件事：注入 `WONTON_MATRIX_ALLOWED_USERS` 时把已成立的联系人加在归属人后面，
//! 名单变了重建这个 Bot 的服务。联系人记录在 [`crate::contacts`]，这里只按 Bot 的用户 ID 问它。

use std::sync::Arc;

use wonton_protocol::{BotKeeper, BotRef};

use super::bots::is_bot;
use super::health_gate::Gate;
use super::manager::{InstallError, PluginManager, PluginStatus};
use super::manifest::PluginId;
use super::render;
use super::upgrade::installed_package;

/// Bot 的 Matrix 用户 ID → 它已成立的联系人。
pub type ContactSource = Arc<dyn Fn(&str) -> Vec<String> + Send + Sync>;

/// 本机的一个 Bot 的记录：插件和 Bot 名、Matrix 用户 ID、归属。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BotListing {
    pub bot: BotRef,
    pub user_id: String,
    pub keeper: BotKeeper,
}

impl PluginManager {
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

    /// 联系人变了：插件在运行时重建这个 Bot 的服务（沿用设备和令牌），名单跟上。插件停着时不动，下次启动时渲染就带上。
    pub async fn rebuild_for_contacts(&self, bot: &BotRef) -> Result<(), InstallError> {
        let id =
            PluginId::parse(&bot.plugin_id).map_err(|e| InstallError::Invalid(e.to_string()))?;
        if self.status_of(&id) != Some(PluginStatus::Running) {
            tracing::info!(plugin = %id, bot = %bot.bot, "联系人变了，插件没在运行，下次启动带上");
            return Ok(());
        }
        let record = self
            .store
            .load(&id)?
            .ok_or_else(|| InstallError::NotInstalled(id.to_string()))?;
        let langs = self.owner_languages();
        let package = installed_package(&record, &langs)?;
        if !is_bot(&package.manifest) {
            return Ok(());
        }
        if package.manifest.bots.is_some() {
            return self.rebuild_bot(&id, &bot.bot).await;
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
