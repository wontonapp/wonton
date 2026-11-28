//! 核心启动时按记录恢复插件：单个插件失败只标异常，不影响核心和其他插件；
//! 以及恢复期间供网关识别的标记。

use std::sync::atomic::Ordering;

use super::super::entry::{Entry, PluginStatus};
use super::super::languages::Languages;
use super::super::listed::Listed;
use super::super::package::PluginPackage;
use super::super::store::InstalledRecord;
use super::{InstallError, PluginManager, Restoring};
use crate::l10n;

impl PluginManager {
    /// 核心启动时：按记录恢复插件状态，把 OIDC 客户端同步到身份提供方。
    /// 单个插件恢复失败（包括它的记录损坏）只把它标为异常，不影响核心和其他插件启动。
    ///
    /// 这里是一个插件接一个插件串行的：漂移重建只等 `install_timeout`，不让一个起不来的慢插件把排在后面
    /// 的所有插件拖住几十分钟（见 [`super::super::health_gate::Gate`]）。超时保留续建账本，后台按间隔重试。
    pub async fn restore(&self) -> Result<(), InstallError> {
        let _restoring = self.restoring_guard();
        let langs = self.owner_languages();
        self.prune_orphaned_bot_web().await?;
        let files = self.store.load_all()?;
        let mut ordered = Vec::with_capacity(files.len());
        for file in files {
            let priority = match &file.record {
                Ok(record) => {
                    match PluginPackage::with_files(&record.source, record.files.clone(), &langs) {
                        Ok(package) => {
                            let provides = package.manifest.provides.as_ref();
                            let priority = if super::super::egress::provider::provides_proxy(
                                &package.manifest,
                            ) {
                                0
                            } else if provides.is_some_and(|provides| {
                                provides.models.is_some() || !provides.capabilities.is_empty()
                            }) {
                                1
                            } else {
                                2
                            };
                            self.plugins.write().expect("插件表锁中毒").insert(
                                file.id.clone(),
                                Entry::new(
                                    Listed::of_record(&package, record),
                                    PluginStatus::Installing,
                                    None,
                                ),
                            );
                            self.forget_bot_sync(&file.id);
                            priority
                        }
                        Err(_) => 2,
                    }
                }
                Err(_) => 2,
            };
            ordered.push((priority, file));
        }
        ordered.sort_by_key(|(priority, _)| *priority);
        for (_, file) in ordered {
            let entry = match &file.record {
                Ok(record) => self.restore_one(record, &langs).await,
                Err(e) => Entry::broken(&file.id, String::new(), unreadable_record(&e, &langs)),
            };
            match &entry.status {
                PluginStatus::Running => tracing::info!(plugin = %file.id, "已恢复插件"),
                PluginStatus::Stopped => tracing::info!(plugin = %file.id, "插件已停止"),
                status => tracing::warn!(plugin = %file.id, ?status, "插件状态异常"),
            }
            self.plugins
                .write()
                .expect("插件表锁中毒")
                .insert(file.id.clone(), entry);
            if let Ok(record) = &file.record
                && let Ok(package) =
                    PluginPackage::with_files(&record.source, record.files.clone(), &langs)
                && let Err(error) = self
                    .capabilities_installed(&package.manifest, &[], true)
                    .await
            {
                tracing::error!(plugin = %file.id, %error, "恢复能力连接失败");
            }
        }
        self.follow_core_proxy();
        self.changed();
        Ok(())
    }

    async fn restore_one(&self, record: &InstalledRecord, langs: &Languages) -> Entry {
        match record.package(langs) {
            Ok(package) if super::super::install_hosted::is_containerless(&package.manifest) => {
                self.restore_containerless(&package, record, langs)
            }
            Ok(package) => {
                // 不管核对结果如何都先登记来源：插件暂时不健康，也要能经核心重新连上 Matrix 恢复过来。
                let manifest = &package.manifest;
                let network = self.cfg.instance.network(&manifest.id.slug());
                if let Err(e) = self.register_source(manifest, &network).await {
                    tracing::warn!(plugin = %record.id, error = %e, "登记插件网络的地址段失败");
                }
                let listed = Listed::of_record(&package, record);
                match self.restore_runtime(&package, record).await {
                    Ok(entry) => entry,
                    Err(error) => Entry::containers_failed(listed, error.to_string()),
                }
            }
            Err(e) => Entry::broken(
                &record.id,
                record.version.clone(),
                unreadable_record(&e, langs),
            ),
        }
    }

    /// 恢复期间为真：网关据此把 Bot 的请求答成可重试的 503，不答 403（上游适配器见 403 就停同步）。
    ///
    /// 数层数而不是一个位：两段恢复叠在一起时，先结束的那段不能把还在进行的那段说成恢复了。
    pub(crate) fn restoring(&self) -> bool {
        self.restoring.load(Ordering::Acquire) > 0
    }

    /// 从现在起 [`Self::restoring`] 为真，落地时清掉。
    pub(crate) fn restoring_guard(&self) -> Restoring<'_> {
        self.restoring.fetch_add(1, Ordering::AcqRel);
        Restoring(self)
    }
}

/// 记录读不懂时给主人看的原因：比如核心升级后不再认记录里的字段。
pub(in crate::plugin) fn unreadable_record(
    error: &dyn std::fmt::Display,
    langs: &Languages,
) -> String {
    l10n::text(
        langs,
        "plugin-install-record-unreadable",
        &[("error", error.to_string().into())],
    )
}
