//! 插件的 Bot（`matrix.bot`，插件规范的“Matrix Bot”）：安装时代 Bot 登录一台设备、设置资料、建好主房间，
//! 注入 `WONTON_MATRIX_*`，插件装好后把主房间交给归属人；卸载时吊销 Bot 的设备，删除数据时停用账号。
//!
//! 记录在插件记录目录下的 `bots/<插件 id>.json`：每个 Bot 一条（[`PluginBots`]），记 Matrix 身份、这次登录的
//! 设备、归属人（[`BotKeeper`]）和总线令牌。不删数据卸载、安装失败回滚都保留，重装时沿用，房间和历史都还在。
//! 只有删除数据时才停用账号、删掉记录。建别的 Bot、删 Bot 的部署在 [`super::bots_lifecycle`]。

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use wonton_protocol::BotKeeper;

use super::languages::Languages;
use super::manager::{BotReport, InstallError, PluginManager};
use super::manifest::{LocalizedText, Manifest, Permission, PluginId};
use super::package::PluginPackage;
use super::package_files;
use super::store::{StoreError, create_private_dir, read_json, remove_file, write_json};
use super::vars::MatrixVars;
use crate::config::PLUGIN_MATRIX_HOST;
use crate::matrix::{Avatar, BotAccount, BotSpec, Matrix, MatrixError, ProvisionedBot};
use crate::secret::random_token;

/// 一个 Bot 的记录：Matrix 身份、正在用的设备、归属人和总线令牌。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BotEntry {
    #[serde(flatten)]
    pub account: BotAccount,
    /// 插件容器正在用的设备；卸载或安装失败回滚后为空。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_id: Option<String>,
    pub keeper: BotKeeper,
    /// 总线令牌，在 `WONTON_MCP_URL` 的路径里（`/mcp/<令牌>`）。建 Bot 时生成，之后不变。
    pub bus_token: String,
}

/// 一个插件的全部 Bot 记录，每个插件一个 JSON 文件（0600）。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginBots {
    #[serde(default)]
    pub bots: BTreeMap<String, BotEntry>,
}

impl PluginBots {
    /// 这个 Bot 的 Matrix 用户 ID。
    pub fn user_id(&self, bot: &str) -> Option<&str> {
        self.bots.get(bot).map(|e| e.account.user_id.as_str())
    }

    /// 这个用户 ID 是不是记录在案的 Bot。
    pub fn has_user(&self, user_id: &str) -> bool {
        self.bots.values().any(|e| e.account.user_id == user_id)
    }
}

/// Bot 记录的存取。
pub struct BotStore {
    dir: PathBuf,
}

impl BotStore {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    fn path(&self, id: &PluginId) -> PathBuf {
        self.dir.join(format!("{id}.json"))
    }

    pub fn load(&self, id: &PluginId) -> Result<PluginBots, StoreError> {
        Ok(read_json::<PluginBots>(&self.path(id))?.unwrap_or_default())
    }

    /// 一个 Bot 的记录。
    pub fn entry(&self, id: &PluginId, bot: &str) -> Result<Option<BotEntry>, StoreError> {
        Ok(self.load(id)?.bots.get(bot).cloned())
    }

    pub fn save(&self, id: &PluginId, bots: &PluginBots) -> Result<(), StoreError> {
        create_private_dir(&self.dir)?;
        write_json(&self.path(id), bots)
    }

    pub fn remove(&self, id: &PluginId) -> Result<(), StoreError> {
        remove_file(&self.path(id))
    }

    /// 全部 Bot 用掉的 Matrix 用户名（localpart）：给家人用户名的查重用（ADR-0045）。
    pub fn localparts(&self) -> Result<BTreeSet<String>, StoreError> {
        Ok(self
            .all()?
            .into_values()
            .flat_map(|bots| bots.bots.into_values())
            .filter_map(|entry| localpart(&entry.account.user_id).map(str::to_owned))
            .collect())
    }

    /// 所有插件的 Bot 记录，按插件 ID 排序。
    pub fn all(&self) -> Result<BTreeMap<PluginId, PluginBots>, StoreError> {
        let entries = match std::fs::read_dir(&self.dir) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
            Err(source) => {
                return Err(StoreError::Io {
                    path: self.dir.clone(),
                    source,
                });
            }
        };
        let mut all = BTreeMap::new();
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let Some(id) = path
                .file_stem()
                .and_then(|s| s.to_str())
                .and_then(|s| PluginId::parse(s).ok())
            else {
                continue;
            };
            let bots: PluginBots = read_json(&path)?.unwrap_or_default();
            all.insert(id, bots);
        }
        Ok(all)
    }

    /// 主房间是 `room` 的 Bot 的用户名，扫所有插件的记录。
    pub fn localparts_in_room(&self, room: &str) -> Result<Vec<String>, StoreError> {
        Ok(self
            .all()?
            .into_values()
            .flat_map(|bots| bots.bots.into_values())
            .filter(|entry| entry.account.home_room == room)
            .filter_map(|entry| localpart(&entry.account.user_id).map(str::to_owned))
            .collect())
    }
}

fn localpart(user_id: &str) -> Option<&str> {
    user_id.strip_prefix('@')?.split_once(':').map(|(l, _)| l)
}

pub(super) fn is_bot(manifest: &Manifest) -> bool {
    manifest.permissions.contains(&Permission::MatrixBot)
}

/// Bot 名（插件规范 `WONTON_BOT_NAME`）：小写字母、数字和单个 `-`，以字母或数字开头，最长 32 个字符。
pub fn check_bot_name(bot: &str) -> Result<(), String> {
    let valid = !bot.is_empty()
        && bot.len() <= 32
        && bot.split('-').all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        });
    if valid {
        Ok(())
    } else {
        Err("Bot 名只能有小写字母、数字和单个 -".to_owned())
    }
}

/// 中文优先，没有中文时用英文。清单的 `title` 和 `bot.display_name` 都要求两种语言都有。
fn pick(text: &LocalizedText) -> Option<&str> {
    ["zh", "en"]
        .iter()
        .find_map(|lang| text.get(*lang))
        .map(String::as_str)
}

/// 显示名：`bot.display_name`，没写时用插件的 `title`。
fn display_name(manifest: &Manifest) -> String {
    manifest
        .bot
        .as_ref()
        .and_then(|b| b.display_name.as_ref())
        .and_then(pick)
        .or_else(|| pick(&manifest.title))
        .unwrap_or(manifest.id.as_str())
        .to_owned()
}

/// `bot.avatar` 指向的插件包里的文件，和按扩展名定的类型。
fn avatar<'a>(
    package: &'a PluginPackage,
    langs: &Languages,
) -> Result<Option<Avatar<'a>>, InstallError> {
    let Some(raw) = package
        .manifest
        .bot
        .as_ref()
        .and_then(|b| b.avatar.as_ref())
    else {
        return Ok(None);
    };
    let path = package_files::normalize(raw).map_err(InstallError::Invalid)?;
    let (filename, data) = package.files().get_key_value(&path).ok_or_else(|| {
        InstallError::Invalid(crate::l10n::text(
            langs,
            "plugin-settings-bot-avatar-missing",
            &[("path", path.as_str().into())],
        ))
    })?;
    let extension = path.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase());
    let content_type = match extension.as_deref() {
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        _ => {
            return Err(InstallError::Invalid(crate::l10n::text(
                langs,
                "plugin-settings-bot-avatar-type",
                &[("path", path.as_str().into())],
            )));
        }
    };
    let filename = filename.rsplit('/').next().unwrap_or(filename);
    Ok(Some(Avatar {
        content_type,
        filename,
        data,
    }))
}

impl From<MatrixError> for InstallError {
    fn from(e: MatrixError) -> Self {
        Self::MatrixUnavailable(e.to_string())
    }
}

/// Bot 的 Matrix 用户名：默认 Bot 是插件 `id` 的最后一段，其余是 `<最后一段>-<Bot 名>`。
fn bot_localpart(id: &PluginId, bot: &str) -> String {
    let last = id.as_str().rsplit('.').next().unwrap_or(id.as_str());
    if bot == super::bot_services::DEFAULT_BOT {
        return last.to_owned();
    }
    format!("{last}-{bot}")
}

impl PluginManager {
    pub(crate) fn matrix(&self) -> Result<&Matrix, InstallError> {
        let langs = self.owner_languages();
        self.matrix.as_deref().ok_or_else(|| {
            InstallError::MatrixUnavailable(crate::l10n::text(
                &langs,
                "plugin-settings-bot-matrix-unavailable",
                &[],
            ))
        })
    }

    /// 准备一个 Bot：代它登录一台新设备，设置资料，准备好主房间，把记录（含归属人和总线令牌）存下来。
    /// 已有记录时沿用账号、主房间和令牌，只换设备。建好后由调用方部署这个 Bot 的容器、把主房间交给
    /// 归属人（共享 Bot 是主人）。
    pub(super) async fn provision_bot(
        &self,
        id: &PluginId,
        manifest: &Manifest,
        bot: &str,
        keeper: BotKeeper,
        package: &PluginPackage,
    ) -> Result<ProvisionedBot, InstallError> {
        // 同时建的两个 Bot 不能分到同一个用户名，也不能同时改 Bot 记录。
        let _allocating = self.bot_allocation.lock().await;
        self.provision_bot_locked(id, manifest, bot, keeper, package)
            .await
    }

    /// 同 [`Self::provision_bot`]，调用方已持有 [`Self::bot_allocation`]。
    pub(super) async fn provision_bot_locked(
        &self,
        id: &PluginId,
        manifest: &Manifest,
        bot: &str,
        keeper: BotKeeper,
        package: &PluginPackage,
    ) -> Result<ProvisionedBot, InstallError> {
        let matrix = self.matrix()?;
        let store = self.store.bots();
        let mut plugin_bots = store.load(id)?;
        // 同一个 Bot 重试安装时沿用账号；其他 Bot 的令牌不变。
        let existing = plugin_bots.bots.get(bot).cloned();
        let mut taken: BTreeSet<String> = store
            .all()?
            .into_values()
            .flat_map(|bots| bots.bots.into_values())
            .filter_map(|entry| localpart(&entry.account.user_id).map(str::to_owned))
            .collect();
        self.reserve_friend_names(&mut taken);
        if let Some(members) = self.person_members.get() {
            taken.extend(members.list().into_iter().map(|member| member.username));
        }
        let langs = self.owner_languages();
        let spec = BotSpec {
            localpart: &bot_localpart(id, bot),
            taken: &taken,
            previous: existing.as_ref().map(|e| &e.account),
            device_name: &crate::l10n::text(
                &langs,
                "plugin-settings-bot-device-name",
                &[("id", id.as_str().into()), ("bot", bot.into())],
            ),
            display_name: &display_name(manifest),
            avatar: avatar(package, &langs)?,
            owner: &self.keeper_user(&keeper),
        };
        let provisioned = matrix.provision_bot(&spec).await.map_err(|e| {
            if e.errcode() == "M_USER_DEACTIVATED" {
                let user = existing.as_ref().map_or("", |e| e.account.user_id.as_str());
                InstallError::Conflict(crate::l10n::text(
                    &langs,
                    "plugin-settings-bot-account-deactivated",
                    &[("user", user.into())],
                ))
            } else {
                e.into()
            }
        })?;
        let bus_token = existing.map_or_else(random_token, |e| e.bus_token);
        plugin_bots.bots.insert(
            bot.to_owned(),
            BotEntry {
                account: provisioned.account.clone(),
                device_id: Some(provisioned.device_id.clone()),
                keeper,
                bus_token,
            },
        );
        store.save(id, &plugin_bots)?;
        self.changed();
        self.initialize_default_avatars(package, &provisioned)
            .await?;
        tracing::info!(
            plugin = %id,
            bot,
            user = %provisioned.account.user_id,
            room = %provisioned.account.home_room,
            "Bot 已就绪"
        );
        Ok(provisioned)
    }

    /// 归属人里主房间交给谁、注入的名单算谁：私人 Bot 是归属人，共享 Bot 是主人（家人加入后名单再变长）。
    pub(super) fn keeper_user(&self, keeper: &BotKeeper) -> String {
        match keeper {
            BotKeeper::User { username } => username.clone(),
            BotKeeper::Household | BotKeeper::Other => self.cfg.owner.clone(),
        }
    }

    /// 注入给一个 Bot 的服务的 `WONTON_MATRIX_*`：经核心在插件网络里的别名访问 Matrix 客户端接口，
    /// `WONTON_MATRIX_ALLOWED_USERS` 是它听的人：归属人和已成立的联系人。
    pub(super) fn matrix_vars(&self, bot: &ProvisionedBot) -> MatrixVars {
        MatrixVars {
            homeserver: format!("http://{PLUGIN_MATRIX_HOST}:{}", self.cfg.gateway_port),
            user_id: bot.account.user_id.clone(),
            access_token: bot.access_token.clone(),
            allowed_users: self.allowed_users(bot.owner.clone(), &bot.account.user_id),
            home_room: bot.account.home_room.clone(),
        }
    }

    /// 把默认 Bot 的身份填进安装上下文：单个 Bot 的插件放插件级变量，声明了 `bots` 的再放按 Bot 的
    /// 变量（安装和升级新建 Bot 后都用它）。
    pub(super) fn fill_default_bot_vars(
        &self,
        manifest: &Manifest,
        bot: &ProvisionedBot,
        ctx: &mut super::render::InstallContext,
    ) {
        ctx.matrix = Some(self.matrix_vars(bot));
        // Bot 的变量先放齐（模型凭据、Matrix 身份），总线地址只有申请了 tools 的插件才有。
        if manifest.bots.is_some() {
            ctx.bots.insert(
                super::bot_services::DEFAULT_BOT.to_owned(),
                super::bot_services::BotVars {
                    model_key: ctx
                        .models
                        .as_ref()
                        .and_then(|m| m.bot_keys.get(super::bot_services::DEFAULT_BOT).cloned()),
                    matrix: ctx.matrix.clone(),
                    mcp_url: None,
                },
            );
        }
        if !manifest.permissions.contains(&Permission::Tools) {
            return;
        }
        let Ok(plugin_bots) = self.store.bots().load(&manifest.id) else {
            return;
        };
        let Some(entry) = plugin_bots.bots.get(super::bot_services::DEFAULT_BOT) else {
            return;
        };
        let url = format!(
            "http://{}:{}/mcp/{}",
            crate::config::PLUGIN_MCP_HOST,
            self.cfg.gateway_port,
            entry.bus_token
        );
        ctx.mcp_url = Some(url.clone());
        if let Some(vars) = ctx.bots.get_mut(super::bot_services::DEFAULT_BOT) {
            vars.mcp_url = Some(url);
        }
    }

    /// 存下来的一个 Bot 的 Matrix 变量：令牌是占位值（重建时从旧容器读回），名单按归属人算
    /// （私人 Bot 是归属人，共享 Bot 是主人）。
    pub(super) fn stored_matrix_vars(&self, entry: &BotEntry) -> Result<MatrixVars, InstallError> {
        let matrix = self.matrix()?;
        let owner = self.keeper_user(&entry.keeper);
        Ok(MatrixVars {
            homeserver: format!("http://{PLUGIN_MATRIX_HOST}:{}", self.cfg.gateway_port),
            user_id: entry.account.user_id.clone(),
            access_token: super::egress::PLACEHOLDER.to_owned(),
            allowed_users: self.allowed_users(matrix.user_id(&owner), &entry.account.user_id),
            home_room: entry.account.home_room.clone(),
        })
    }

    /// 插件装好、健康检查通过后把主房间交给归属人（共享 Bot 是主人，`owner` 是它的用户名），返回这次是否
    /// 交出了；对方已经在里面（重装）时返回 false。早交出去的话，对方在 Bot 启动前发的话它收不到。
    pub(super) async fn hand_over_home_room(
        &self,
        owner: &str,
        bot: &ProvisionedBot,
    ) -> Result<bool, InstallError> {
        let langs = self.owner_languages();
        Ok(self.matrix()?.hand_over(owner, bot, &langs).await?)
    }

    /// 安装在交出主房间之后失败了：收回它，归属人那边不留一个没有 Bot 应答的私聊。
    pub(super) async fn withdraw_home_room(&self, bot: &ProvisionedBot) -> Result<(), String> {
        let matrix = self.matrix().map_err(|e| e.to_string())?;
        let langs = self.owner_languages();
        matrix
            .withdraw(bot, &langs)
            .await
            .map_err(|e| e.to_string())
    }

    /// 安装失败回滚：吊销这次登录的设备，账号和主房间留给下次安装。
    pub(super) async fn revoke_bot_device(
        &self,
        id: &PluginId,
        provisioned: &ProvisionedBot,
    ) -> Result<(), String> {
        let matrix = self.matrix().map_err(|e| e.to_string())?;
        matrix
            .delete_device(&provisioned.account.user_id, &provisioned.device_id)
            .await
            .map_err(|e| e.to_string())?;
        // 记录里按账号找这台设备：调用方（如头像初始化失败）手里只有 ProvisionedBot。
        let mut plugin_bots = self.store.bots().load(id).map_err(|e| e.to_string())?;
        if let Some(entry) = plugin_bots
            .bots
            .values_mut()
            .find(|entry| entry.account.user_id == provisioned.account.user_id)
        {
            entry.device_id = None;
            self.store
                .bots()
                .save(id, &plugin_bots)
                .map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    /// 卸载：吊销所有 Bot 的设备；删除数据时再停用账号（不可恢复）、删掉记录。
    pub(super) async fn deactivate_bots(
        &self,
        id: &PluginId,
        purge: bool,
    ) -> Result<Vec<BotReport>, InstallError> {
        let store = self.store.bots();
        let plugin_bots = store.load(id)?;
        if plugin_bots.bots.is_empty() {
            return Ok(Vec::new());
        }
        let matrix = self.matrix()?;
        let mut reports = Vec::new();
        for entry in plugin_bots.bots.values() {
            let user = &entry.account.user_id;
            let devices = matrix.devices(user).await?;
            for device in &devices {
                matrix.delete_device(user, device).await?;
            }
            if purge {
                matrix.deactivate(user).await?;
            }
            reports.push(BotReport {
                user_id: user.clone(),
                devices_removed: devices.len(),
                deactivated: purge,
            });
        }
        if purge {
            store.remove(id)?;
        } else {
            let kept = plugin_bots
                .bots
                .into_iter()
                .map(|(bot, entry)| {
                    (
                        bot,
                        BotEntry {
                            device_id: None,
                            ..entry
                        },
                    )
                })
                .collect();
            store.save(id, &PluginBots { bots: kept })?;
        }
        Ok(reports)
    }

    /// 核心重启后核对 Bot：每个插件拿着的令牌对应的设备还在。被吊销了就如实报异常，重装才能重新签发。
    pub(super) async fn check_bot(&self, manifest: &Manifest) -> Result<(), InstallError> {
        if !is_bot(manifest) {
            return Ok(());
        }
        let matrix = self.matrix()?;
        let plugin_bots = self.store.bots().load(&manifest.id)?;
        for (bot, entry) in &plugin_bots.bots {
            let Some(device) = &entry.device_id else {
                return Err(InstallError::Unhealthy(crate::l10n::text(
                    &self.owner_languages(),
                    "plugin-settings-bot-device-record-missing",
                    &[("bot", bot.as_str().into())],
                )));
            };
            let user = &entry.account.user_id;
            if !matrix.devices(user).await?.contains(device) {
                return Err(InstallError::Unhealthy(crate::l10n::text(
                    &self.owner_languages(),
                    "plugin-settings-bot-device-revoked",
                    &[("user", user.as_str().into())],
                )));
            }
        }
        Ok(())
    }

    /// 这个房间里的 Bot 的名字（Matrix 用户名）。M1 的 Bot 只在自己的主房间里，核心不在别的房间里，也查不了成员。
    pub fn bots_in_room(&self, room: &str) -> Result<Vec<String>, StoreError> {
        self.store.bots().localparts_in_room(room)
    }

    /// Matrix 用户 ID 是哪个插件的哪个 Bot，卡片发给谁：私人 Bot 是归属人，共享 Bot 是主人。认不出时是 `None`。
    pub fn bot_by_user(&self, user_id: &str) -> Option<(PluginId, String, String)> {
        let all = self
            .store
            .bots()
            .all()
            .inspect_err(|error| tracing::warn!(error = %error, "读 Bot 记录失败"))
            .ok()?;
        all.into_iter().find_map(|(id, bots)| {
            bots.bots
                .into_iter()
                .find(|(_, entry)| entry.account.user_id == user_id)
                .map(|(bot, entry)| (id, bot, self.keeper_user(&entry.keeper)))
        })
    }

    /// 总线令牌认出的这个插件的 Bot：令牌对不上就是没有。
    pub fn bot_by_token(&self, id: &PluginId, token: &str) -> Option<(String, BotKeeper)> {
        let bots = self.store.bots().load(id).ok()?;
        bots.bots
            .into_iter()
            .find(|(_, entry)| crate::secret::constant_time_eq(&entry.bus_token, token))
            .map(|(bot, entry)| (bot, entry.keeper))
    }

    /// 一个 Bot 的 Matrix 账号和主房间；没有这个 Bot 的记录、或记录读不出来时为空。
    pub fn bot_account(&self, id: &PluginId, bot: &str) -> Option<BotAccount> {
        let bots = self
            .store
            .bots()
            .load(id)
            .inspect_err(|error| tracing::warn!(plugin = %id, %error, "读 Bot 记录失败"))
            .ok()?;
        bots.bots.get(bot).map(|entry| entry.account.clone())
    }

    /// 一个 Bot 的 Matrix 用户名（localpart）；没有这个 Bot 的记录时为空。
    pub fn bot_localpart(&self, id: &PluginId, bot: &str) -> Result<Option<String>, StoreError> {
        let bots = self.store.bots().load(id)?;
        Ok(bots
            .bots
            .get(bot)
            .and_then(|entry| localpart(&entry.account.user_id).map(str::to_owned)))
    }

    /// 记录在案的 Bot 名，按名字排序。
    pub(super) fn bot_names(&self, id: &PluginId) -> Vec<String> {
        self.store
            .bots()
            .load(id)
            .map(|bots| bots.bots.keys().cloned().collect())
            .unwrap_or_default()
    }

    /// 这个插件有没有 Bot 记录：有的插件调用总线必须带令牌认 Bot。
    pub fn has_bots(&self, id: &PluginId) -> bool {
        self.store
            .bots()
            .load(id)
            .map(|bots| !bots.bots.is_empty())
            .unwrap_or(false)
    }

    /// 这个用户 ID 是不是这个插件自己的 Bot：Bot 的 Matrix 入口用它放行本插件的 Bot、拒绝别人的令牌。
    pub fn is_bot_user(&self, id: &PluginId, user_id: &str) -> bool {
        self.store
            .bots()
            .load(id)
            .map(|bots| bots.has_user(user_id))
            .unwrap_or(false)
    }
}

#[cfg(test)]
impl PluginManager {
    /// 测试用：直接写 Bot 记录（网关和总线的测试不装插件，只认令牌）。
    pub(crate) fn bot_records_for_test(
        &self,
        id: &PluginId,
        entries: &[(&str, &str, BotKeeper, &str)],
    ) {
        let bots = entries
            .iter()
            .map(|(bot, user_id, keeper, token)| {
                (
                    bot.to_string(),
                    BotEntry {
                        account: BotAccount {
                            user_id: user_id.to_string(),
                            home_room: format!("!{bot}:test"),
                        },
                        device_id: None,
                        keeper: keeper.clone(),
                        bus_token: token.to_string(),
                    },
                )
            })
            .collect();
        self.store.bots().save(id, &PluginBots { bots }).unwrap();
    }
}
