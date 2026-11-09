//! Bot 碰文件的两件事（ADR-0020 的“Bot”）：工具结果里的引用登记成编号，调工具时核对编号还用得了。

use std::sync::Arc;

use wonton_protocol::{FileFailure, FileId, FileSource};

use super::record::{Accessor, BOT_TTL_SECS, FileRecord, Origin};
use super::resource::InternalUrl;
use super::share_fs::{self, ShareRef, sanitize_name};
use super::{BotRef, Failure, Files, Scale, new_id};
use crate::l10n;
use crate::plugin::PluginId;
use crate::plugin::languages::Languages;
use crate::plugin::manager::now_secs;

impl Files {
    /// Bot 经工具结果登记共享文件夹里的文件。来源插件要有这个共享文件夹的授权。同一个 Bot 再次拿到同一个引用，还是原来的编号，有效期
    /// 重新算；分享过的换了插件再给时发新编号（见 [`Registry::register_bot`](super::record::Registry::register_bot)）。
    pub async fn register_share(
        self: &Arc<Self>,
        bot: &BotRef,
        reference: ShareRef,
        name: Option<&str>,
        mime_type: Option<&str>,
        langs: &Languages,
    ) -> Result<FileRecord, Failure> {
        self.require_share(&bot.plugin, &reference.share, langs)?;
        let bot = bot.clone();
        let name = sanitize_name(name.unwrap_or_else(|| reference.file_name()));
        let mime_type = mime_type.map(str::to_owned);
        self.blocking(move |files| {
            let snapshot = files.shares.open(&reference)?.snapshot;
            let origin = Origin::Share { reference };
            files.register(&bot, origin, name, mime_type, None, Some(snapshot))
        })
        .await
    }

    /// Bot 经工具结果登记插件的资源。内网地址只认来源插件自己的服务（ADR-0020 的“鉴权”）。
    #[allow(clippy::too_many_arguments)] // 一个 `resource_link` 的几个字段，再包一层也是逐个填。
    pub async fn register_resource(
        self: &Arc<Self>,
        bot: &BotRef,
        plugin: &PluginId,
        uri: &str,
        internal_url: Option<&str>,
        name: &str,
        mime_type: Option<&str>,
        size: Option<u64>,
        langs: &Languages,
    ) -> Result<FileRecord, Failure> {
        if let Some(url) = internal_url {
            let parsed = InternalUrl::parse(url, langs)
                .map_err(|e| Failure::new(FileFailure::PermissionDenied, e))?;
            self.host.service_upstream(plugin, &parsed).await?;
        }
        let origin = Origin::Plugin {
            plugin: plugin.clone(),
            uri: uri.to_owned(),
            internal_url: internal_url.map(str::to_owned),
        };
        let bot = bot.clone();
        let name = sanitize_name(name);
        let mime_type = mime_type.map(str::to_owned);
        self.blocking(move |files| files.register(&bot, origin, name, mime_type, size, None))
            .await
    }

    fn register(
        &self,
        bot: &BotRef,
        origin: Origin,
        name: String,
        mime_type: Option<String>,
        declared_size: Option<u64>,
        registered: Option<share_fs::Snapshot>,
    ) -> Result<FileRecord, Failure> {
        let now = now_secs();
        let expires_at = now + BOT_TTL_SECS;
        let looked_up = origin.clone();
        let source = FileSource::Bot {
            name: bot.name.clone(),
            plugin: bot.plugin.as_str().to_owned(),
        };
        let renewed = source.clone();
        let record = self.registry.register_bot(
            &bot.name,
            &looked_up,
            bot.plugin.as_str(),
            now,
            |old| {
                // Bot 重新看到了这个文件，改过的也不算改过了，单独收回过的也回到范围里。主人亲手的授权不恢复：之后按来源插件的授权算。
                // 没分享的之后核对授权看这次给它引用的插件；分享过的不再过期，来源本来就是这个插件。
                old.registered = registered;
                old.access.insert(Accessor::Bot(bot.name.clone()));
                if old.expires_at.is_some() {
                    old.expires_at = Some(expires_at);
                    old.source = renewed;
                }
            },
            || FileRecord {
                id: new_id(),
                name,
                mime_type,
                declared_size,
                source,
                origin,
                access: [Accessor::Bot(bot.name.clone())].into(),
                owner_granted: Default::default(),
                registered,
                created_at: now,
                expires_at: Some(expires_at),
            },
        )?;
        Ok(record)
    }

    /// Bot 调工具时查编号：不在范围里、过期了，还有这之后才出的问题（来源插件的授权收回了、文件改过或者没了）。主人亲手授权的 Bot
    /// 不核对来源插件的授权，也不核对文件改没改过：这是主人给的文件，读得到就行。
    pub async fn for_bot(
        &self,
        id: &FileId,
        bot: &str,
        langs: &Languages,
    ) -> Result<FileRecord, Failure> {
        let record = self.authorize(id, &Accessor::Bot(bot.to_owned()), langs)?;
        match &record.origin {
            Origin::Share { reference } if record.owner_granted.contains(bot) => {
                self.open_share(reference.clone(), langs).await?;
            }
            Origin::Share { reference } => {
                if let FileSource::Bot { plugin, .. } = &record.source {
                    let plugin = PluginId::parse(plugin)
                        .map_err(|e| Failure::internal(format!("记录里的插件 ID 不对：{e}")))?;
                    self.require_share(&plugin, &reference.share, langs)?;
                }
                let now = self.open_share(reference.clone(), langs).await?.snapshot;
                let changed = record.registered.is_some_and(|then| {
                    then.size != now.size || then.modified_ns != now.modified_ns
                });
                if changed {
                    return Err(Failure::new(
                        FileFailure::FileChanged,
                        l10n::text(langs, "files-changed", &[]),
                    ));
                }
            }
            Origin::Upload { reference, .. } => {
                self.open_share(reference.clone(), langs).await?;
            }
            Origin::Plugin { plugin, .. } => self.host.check_plugin(plugin)?,
        }
        Ok(record)
    }

    fn require_share(
        &self,
        plugin: &PluginId,
        share: &str,
        langs: &Languages,
    ) -> Result<(), Failure> {
        if self.host.share_granted(plugin, share)? {
            return Ok(());
        }
        Err(Failure::new(
            FileFailure::PermissionDenied,
            l10n::text(
                langs,
                "files-share-not-authorized",
                &[("plugin", plugin.as_str().into()), ("share", share.into())],
            ),
        ))
    }

    /// 给 Bot 看的预览：长边 1024 像素的 JPEG，不带原文件的元数据。
    pub async fn preview(
        &self,
        record: &FileRecord,
        langs: &Languages,
    ) -> Result<Vec<u8>, Failure> {
        self.scaled(record, Scale::Preview, langs).await
    }

    /// Bot 把文件分享给主人：加进范围，不再过期。
    pub async fn share_with_owner(
        self: &Arc<Self>,
        id: &FileId,
        bot: &str,
        langs: &Languages,
    ) -> Result<FileRecord, Failure> {
        self.authorize(id, &Accessor::Bot(bot.to_owned()), langs)?;
        let (id, langs) = (id.clone(), langs.clone());
        self.blocking(move |files| {
            files
                .registry
                .grant(&id, Accessor::Owner, now_secs())?
                .ok_or_else(|| Failure::unknown(&langs))
        })
        .await
    }
}
