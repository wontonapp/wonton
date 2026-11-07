//! 插件的 Bot（`matrix.bot`，插件规范的“Matrix Bot”）：安装时代 Bot 登录一台设备、设置资料、建好主房间，
//! 注入 `WONTON_MATRIX_*`，插件装好后把主房间交给主人；卸载时吊销 Bot 的设备，删除数据时停用账号。
//!
//! 账号和主房间记在插件记录目录下的 `bots/<插件 id>.json`：不删数据卸载、安装失败回滚都保留，重装时沿用，
//! 房间和历史都还在。只有删除数据时才停用账号、删掉记录。

use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::languages::Languages;
use super::manager::{BotReport, InstallError, PluginManager};
use super::manifest::{LocalizedText, Manifest, Permission, PluginId};
use super::package::PluginPackage;
use super::package_files;
use super::store::{StoreError, create_private_dir, read_json, remove_file, write_json};
use super::vars::MatrixVars;
use crate::config::PLUGIN_MATRIX_HOST;
use crate::matrix::{Avatar, BotAccount, BotSpec, Matrix, MatrixError, ProvisionedBot};

/// 一个插件的 Bot 在 Matrix 上的身份。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BotRecord {
    #[serde(flatten)]
    pub account: BotAccount,
    /// 插件容器正在用的设备；卸载或安装失败回滚后为空。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_id: Option<String>,
}

/// Bot 记录，每个插件一个 JSON 文件（0600）。
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

    pub fn load(&self, id: &PluginId) -> Result<Option<BotRecord>, StoreError> {
        read_json(&self.path(id))
    }

    pub fn save(&self, id: &PluginId, record: &BotRecord) -> Result<(), StoreError> {
        create_private_dir(&self.dir)?;
        write_json(&self.path(id), record)
    }

    pub fn remove(&self, id: &PluginId) -> Result<(), StoreError> {
        remove_file(&self.path(id))
    }

    /// 除 `id` 以外的插件的 Bot 用掉的用户名。
    fn localparts_except(&self, id: &PluginId) -> Result<BTreeSet<String>, StoreError> {
        Ok(self
            .records_except(Some(id))?
            .iter()
            .filter_map(|record| localpart(&record.account.user_id).map(str::to_owned))
            .collect())
    }

    /// 全部 Bot 用掉的 Matrix 用户名（localpart）：给家人用户名的查重用（ADR-0045）。
    pub fn localparts(&self) -> Result<BTreeSet<String>, StoreError> {
        Ok(self
            .records_except(None)?
            .iter()
            .filter_map(|record| localpart(&record.account.user_id).map(str::to_owned))
            .collect())
    }

    /// 主房间是 `room` 的 Bot 的用户名。
    pub fn in_home_room(&self, room: &str) -> Result<Vec<String>, StoreError> {
        Ok(self
            .records_except(None)?
            .iter()
            .filter(|record| record.account.home_room == room)
            .filter_map(|record| localpart(&record.account.user_id).map(str::to_owned))
            .collect())
    }

    fn records_except(&self, id: Option<&PluginId>) -> Result<Vec<BotRecord>, StoreError> {
        let entries = match fs::read_dir(&self.dir) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(source) => {
                return Err(StoreError::Io {
                    path: self.dir.clone(),
                    source,
                });
            }
        };
        let mut records = Vec::new();
        for entry in entries {
            let path = entry
                .map_err(|source| StoreError::Io {
                    path: self.dir.clone(),
                    source,
                })?
                .path();
            let own =
                id.is_some_and(|id| path.file_stem().and_then(|s| s.to_str()) == Some(id.as_str()));
            if own || path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            records.extend(read_json::<BotRecord>(&path)?);
        }
        Ok(records)
    }
}

fn localpart(user_id: &str) -> Option<&str> {
    user_id.strip_prefix('@')?.split_once(':').map(|(l, _)| l)
}

pub(super) fn is_bot(manifest: &Manifest) -> bool {
    manifest.permissions.contains(&Permission::MatrixBot)
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

impl PluginManager {
    pub(super) fn matrix(&self) -> Result<&Matrix, InstallError> {
        let langs = self.owner_languages();
        self.matrix.as_deref().ok_or_else(|| {
            InstallError::MatrixUnavailable(crate::l10n::text(
                &langs,
                "plugin-settings-bot-matrix-unavailable",
                &[],
            ))
        })
    }

    /// 声明了 `matrix.bot` 的插件：代 Bot 登录一台新设备，设置资料，准备好主房间，记下账号和设备。
    pub(super) async fn provision_bot(
        &self,
        package: &PluginPackage,
    ) -> Result<Option<ProvisionedBot>, InstallError> {
        let manifest = &package.manifest;
        if !is_bot(manifest) {
            return Ok(None);
        }
        let matrix = self.matrix()?;
        let id = &manifest.id;
        // 同时安装的两个插件不能分到同一个用户名。
        let _allocating = self.bot_allocation.lock().await;
        let store = self.store.bots();
        let previous = store.load(id)?;
        let taken = store.localparts_except(id)?;
        let localpart = id.as_str().rsplit('.').next().unwrap_or(id.as_str());
        let langs = self.owner_languages();
        let spec = BotSpec {
            localpart,
            taken: &taken,
            previous: previous.as_ref().map(|r| &r.account),
            device_name: &crate::l10n::text(
                &langs,
                "plugin-settings-bot-device-name",
                &[("id", id.as_str().into())],
            ),
            display_name: &display_name(manifest),
            avatar: avatar(package, &langs)?,
            owner: &self.cfg.owner,
        };
        let bot = matrix.provision_bot(&spec).await.map_err(|e| {
            if e.errcode() == "M_USER_DEACTIVATED" {
                let user = previous.as_ref().map_or("", |r| r.account.user_id.as_str());
                InstallError::Conflict(crate::l10n::text(
                    &langs,
                    "plugin-settings-bot-account-deactivated",
                    &[("user", user.into())],
                ))
            } else {
                e.into()
            }
        })?;
        let record = BotRecord {
            account: bot.account.clone(),
            device_id: Some(bot.device_id.clone()),
        };
        store.save(id, &record)?;
        self.initialize_default_avatars(package, &bot).await?;
        self.changed();
        tracing::info!(plugin = %id, user = %bot.account.user_id, room = %bot.account.home_room, "Bot 已就绪");
        Ok(Some(bot))
    }

    /// 这个插件的 Bot 的名字（Matrix 用户名）；它没有 Bot 时为空。
    pub fn bot_name(&self, id: &PluginId) -> Result<Option<String>, StoreError> {
        let record = self.store.bots().load(id)?;
        Ok(record.and_then(|r| localpart(&r.account.user_id).map(str::to_owned)))
    }

    /// 这个房间里的 Bot 的名字（Matrix 用户名）。M1 的 Bot 只在自己的主房间里，核心不在别的房间里，也查不了成员。
    pub fn bots_in_room(&self, room: &str) -> Result<Vec<String>, StoreError> {
        self.store.bots().in_home_room(room)
    }

    /// 注入给插件的 `WONTON_MATRIX_*`：经核心在插件网络里的别名访问 Matrix 客户端接口。
    pub(super) fn matrix_vars(&self, bot: &ProvisionedBot) -> MatrixVars {
        MatrixVars {
            homeserver: format!("http://{PLUGIN_MATRIX_HOST}:{}", self.cfg.gateway_port),
            user_id: bot.account.user_id.clone(),
            access_token: bot.access_token.clone(),
            owner: bot.owner.clone(),
            home_room: bot.account.home_room.clone(),
        }
    }

    /// 插件装好、健康检查通过后把主房间交给主人，返回这次是否交出了；主人已经在里面（重装）时返回 false。
    /// 早交出去的话，主人在 Bot 启动前发的话它收不到。
    pub(super) async fn hand_over_home_room(
        &self,
        bot: &ProvisionedBot,
    ) -> Result<bool, InstallError> {
        let langs = self.owner_languages();
        Ok(self
            .matrix()?
            .hand_over(&self.cfg.owner, bot, &langs)
            .await?)
    }

    /// 安装在交出主房间之后失败了：收回它，主人那边不留一个没有 Bot 应答的私聊。
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
        bot: &ProvisionedBot,
    ) -> Result<(), String> {
        let matrix = self.matrix().map_err(|e| e.to_string())?;
        matrix
            .delete_device(&bot.account.user_id, &bot.device_id)
            .await
            .map_err(|e| e.to_string())?;
        let record = BotRecord {
            account: bot.account.clone(),
            device_id: None,
        };
        self.store
            .bots()
            .save(id, &record)
            .map_err(|e| e.to_string())
    }

    /// 卸载：吊销 Bot 的所有设备；删除数据时再停用账号（不可恢复）、删掉记录。
    pub(super) async fn remove_bot(
        &self,
        id: &PluginId,
        purge: bool,
    ) -> Result<Option<BotReport>, InstallError> {
        let store = self.store.bots();
        let Some(record) = store.load(id)? else {
            return Ok(None);
        };
        let matrix = self.matrix()?;
        let user = &record.account.user_id;
        let devices = matrix.devices(user).await?;
        for device in &devices {
            matrix.delete_device(user, device).await?;
        }
        if purge {
            matrix.deactivate(user).await?;
            store.remove(id)?;
        } else {
            let kept = BotRecord {
                device_id: None,
                ..record.clone()
            };
            store.save(id, &kept)?;
        }
        Ok(Some(BotReport {
            user_id: user.clone(),
            devices_removed: devices.len(),
            deactivated: purge,
        }))
    }

    /// 核心重启后核对 Bot：插件拿着的令牌对应的设备还在。被吊销了就如实报异常，重装才能重新签发。
    pub(super) async fn check_bot(&self, manifest: &Manifest) -> Result<(), InstallError> {
        if !is_bot(manifest) {
            return Ok(());
        }
        let matrix = self.matrix()?;
        let record = self.store.bots().load(&manifest.id)?;
        let Some((user, device)) = record
            .as_ref()
            .and_then(|r| Some((&r.account.user_id, r.device_id.as_ref()?)))
        else {
            let langs = self.owner_languages();
            return Err(InstallError::Unhealthy(crate::l10n::text(
                &langs,
                "plugin-settings-bot-device-record-missing",
                &[],
            )));
        };
        if matrix.devices(user).await?.contains(device) {
            Ok(())
        } else {
            Err(InstallError::Unhealthy(crate::l10n::text(
                &self.owner_languages(),
                "plugin-settings-bot-device-revoked",
                &[("user", user.as_str().into())],
            )))
        }
    }
}
