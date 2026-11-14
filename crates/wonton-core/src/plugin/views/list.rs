//! 已安装插件的列表视图：列表、列表项和状态计算。

use std::sync::Arc;

use wonton_protocol::{
    InstalledPlugin, InstalledPlugins, PluginChat, PluginHealth, PluginKind as Kind, PluginLimits,
    PluginState, PluginTab, PluginTrust,
};

use super::super::gallery::IconSource;
use super::super::github_source::Origin;
use super::super::languages::Languages;
use super::super::listed::Listed;
use super::super::manager::{Entry, Health, PluginManager, PluginStatus};
use super::super::manifest::{Manifest, PluginId};
use super::super::manifest_tabs;
use super::text::{kind, trust};

impl PluginManager {
    /// 已安装的插件：系统插件在前，其余按安装的先后。
    pub fn installed(&self, langs: &Languages) -> InstalledPlugins {
        let plugins = self.plugins.read().expect("插件表锁中毒");
        let mut listed: Vec<_> = plugins
            .iter()
            .map(|(id, entry)| {
                let installed_at = entry.listed.as_ref().map_or(0, |l| l.installed_at);
                (
                    (!entry.system, installed_at),
                    self.listing(&plugins, id, entry, langs),
                )
            })
            .chain(
                self.failed_installs
                    .read()
                    .expect("安装结果锁中毒")
                    .iter()
                    .map(|(id, failed)| {
                        (
                            (true, failed.entry.checked_at),
                            self.failed_listing(&plugins, id, failed, langs),
                        )
                    }),
            )
            .collect();
        drop(plugins);
        listed.sort_by(|(a, x), (b, y)| a.cmp(b).then_with(|| x.id.cmp(&y.id)));
        InstalledPlugins {
            plugins: listed
                .into_iter()
                .map(|(_, plugin)| self.with_dependency_status(self.with_bot(plugin), langs))
                .collect(),
            upgrade_check: self.upgrades.problem(),
        }
    }

    /// 一个已安装的插件；没装时为空。
    pub fn installed_plugin(&self, id: &PluginId, langs: &Languages) -> Option<InstalledPlugin> {
        let plugins = self.plugins.read().expect("插件表锁中毒");
        let plugin = match plugins.get(id) {
            Some(entry) => self.listing(&plugins, id, entry, langs),
            None => {
                let failed = self.failed_installs.read().expect("安装结果锁中毒");
                return Some(self.failed_listing(&plugins, id, failed.get(id)?, langs));
            }
        };
        drop(plugins);
        Some(self.with_dependency_status(self.with_bot(plugin), langs))
    }

    /// 列表和详情里的状态：运行中、但必填的用户填写密钥还没填的是“待配置”（ADR-0064）。
    /// 只覆盖运行中：异常、停下、正在安装时那几种原因更要紧。
    fn plugin_state(
        &self,
        id: &PluginId,
        status: PluginState,
        manifest: Option<&Manifest>,
    ) -> PluginState {
        if matches!(status, PluginState::Running { .. })
            && !self.unfilled_secrets(id, manifest).is_empty()
        {
            return PluginState::NeedsConfiguration;
        }
        status
    }

    fn with_dependency_status(
        &self,
        mut plugin: InstalledPlugin,
        langs: &Languages,
    ) -> InstalledPlugin {
        if !matches!(plugin.status, PluginState::Running { .. }) {
            return plugin;
        }
        let listed = self
            .plugins
            .read()
            .expect("插件表锁中毒")
            .iter()
            .find(|(id, _)| id.as_str() == plugin.id)
            .and_then(|(_, entry)| entry.listed.clone());
        if let Some(listed) = listed
            && let Some(reason) = self.dependency_problem(&listed.manifest, langs)
        {
            plugin.status = PluginState::Degraded { reason };
        }
        plugin
    }

    /// 列表里的一项，还没填 Bot。
    pub(in crate::plugin) fn listing(
        &self,
        plugins: &std::collections::BTreeMap<PluginId, Entry>,
        id: &PluginId,
        entry: &Entry,
        langs: &Languages,
    ) -> InstalledPlugin {
        let manifest = entry.listed.as_ref().map(|l| &l.manifest);
        let origin = entry.listed.as_ref().and_then(|l| l.origin.as_ref());
        let (upgrade, mut notices) =
            self.upgrades
                .listing(id, &entry.version, &entry.status, langs);
        let tabs = entry
            .listed
            .as_ref()
            .and_then(|listed| listed.manifest.tabs.as_ref().map(|tabs| (listed, tabs)))
            .map(|(listed, tabs)| {
                tabs.iter()
                    .zip(&listed.tab_icons)
                    .filter_map(|(tab, (icon, selected))| {
                        Some(PluginTab {
                            id: tab.id.clone(),
                            title: langs.pick_owned(&tab.title),
                            icon: icon.as_ref()?.tab_image(),
                            icon_selected: selected.as_ref().map(IconSource::tab_image),
                            path: tab.path.clone(),
                            layout: manifest_tabs::layout(&tab.layout)?,
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        if entry.status == PluginStatus::Running {
            notices.extend(self.oom_notices(id, langs));
        }
        InstalledPlugin {
            chat: manifest
                .filter(|m| super::super::bots::is_bot(m))
                .map(|m| PluginChat {
                    form: m.bot.as_ref().map_or_default(|bot| bot.chat_layout),
                }),
            id: id.to_string(),
            title: langs.pick_owned(&entry.title),
            summary: manifest.map_or_else(String::new, |m| langs.pick_owned(&m.description)),
            version: entry.version.clone(),
            revision: entry
                .listed
                .as_ref()
                .map_or(&entry.version, |listed| &listed.revision)
                .clone(),
            kind: manifest.map_or(Kind::Other, |m| kind(m.kind)),
            runtime: manifest.map_or(wonton_protocol::PluginRuntime::Other, |m| m.runtime.into()),
            icon: entry
                .listed
                .as_ref()
                .and_then(|l| l.icon.as_ref())
                .map(IconSource::paths),
            trust: if entry.system {
                PluginTrust::System
            } else {
                trust(origin)
            },
            source: origin.map(Origin::source),
            provider: origin.map(Origin::provider),
            via_index: origin.and_then(Origin::via_index),
            written_by: None,
            status: self.plugin_state(
                id,
                state(
                    &super::super::install_hosted::effective_status(plugins, entry, langs),
                    entry.health,
                ),
                manifest,
            ),
            web_url: self.open_url(id, entry),
            link_url: manifest
                .and_then(|m| m.link.as_ref())
                .map(|link| link.url.clone()),
            cross_origin_isolated: manifest
                .and_then(|m| m.webui.as_ref())
                .is_some_and(|webui| webui.cross_origin_isolated),
            declares_tools: manifest.is_some_and(|m| m.mcp.is_some() || m.remote_mcp.is_some()),
            tabs,
            bot_user_id: None,
            upgrade,
            removable: !entry.system,
            notices,
            enabled_root_permissions: Vec::new(),
            install_progress: entry.install_progress.clone(),
            install_failure: None,
            tunnel_ports: manifest.and_then(|m| m.tunnel.as_ref()).map_or_else(
                Vec::new,
                |tunnel| {
                    tunnel
                        .ports
                        .iter()
                        .map(|spec| wonton_protocol::MappedPort {
                            port: spec.port,
                            protocol: spec.protocol,
                        })
                        .collect()
                },
            ),
            matrix_widget: manifest.and_then(super::super::manifest_widget::plugin_widget),
        }
    }

    pub(in crate::plugin) fn set_listed_limits(&self, id: &PluginId, limits: PluginLimits) {
        let mut plugins = self.plugins.write().expect("插件表锁中毒");
        if let Some(entry) = plugins.get_mut(id)
            && let Some(listed) = entry.listed.clone()
        {
            entry.listed = Some(Arc::new(Listed {
                limits,
                ..(*listed).clone()
            }));
        }
    }
}

fn state(status: &PluginStatus, health: Option<Health>) -> PluginState {
    match status {
        PluginStatus::Installing => PluginState::Installing,
        PluginStatus::Stopped => PluginState::Stopped,
        PluginStatus::Running => PluginState::Running {
            health: match health {
                Some(Health::Healthy) => PluginHealth::Healthy,
                Some(Health::Unknown) | None => PluginHealth::Unknown,
            },
        },
        PluginStatus::Degraded { reason } => PluginState::Degraded {
            reason: reason.clone(),
        },
        PluginStatus::Uninstalling => PluginState::Uninstalling,
        PluginStatus::Upgrading => PluginState::Upgrading,
    }
}
