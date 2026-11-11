//! 已安装插件的展示信息：装上时从插件包里取出，放在内存的插件表里，不用每次重读插件包。

use std::sync::Arc;

use wonton_protocol::{PluginLimits, PortSpec};

use super::gallery::IconSource;
use super::github_source::Origin;
use super::lan::webui::{self, Switched};
use super::manifest::Manifest;
use super::package::PluginPackage;
use super::store::InstalledRecord;

/// 已安装插件在列表和详情页里显示的信息，装上时从插件包里取出。
#[derive(Debug, Clone)]
pub struct Listed {
    pub manifest: Manifest,
    /// 包内容摘要只在构建展示信息时计算，不随列表轮询重复散列。
    pub revision: String,
    pub icon: Option<IconSource>,
    /// 每项 tabs 的普通图标和选中图标原文件。
    pub tab_icons: Vec<(Option<IconSource>, Option<IconSource>)>,
    /// 装上的时间（Unix 秒），列表按它排序。
    pub installed_at: u64,
    /// 按来源装的从哪里来。
    pub origin: Option<Origin>,
    /// 要开关 `lan_webui` 才开的局域网端口（见 [`webui`]）。
    pub lan_switched: Option<Switched>,
    pub limits: PluginLimits,
    pub lan_webui: bool,
    /// 直连宿主网络的网页连不上过，见 [`super::host_relay`]。
    pub firewalled: bool,
}

impl Listed {
    pub fn new(
        package: &PluginPackage,
        installed_at: u64,
        lan_webui: bool,
        firewalled: bool,
        limits: PluginLimits,
    ) -> Arc<Self> {
        let tab_icons = package
            .manifest
            .tabs
            .as_deref()
            .unwrap_or_default()
            .iter()
            .map(|tab| {
                (
                    tab.icon
                        .as_deref()
                        .and_then(|path| IconSource::of_path(package, path)),
                    tab.icon_selected
                        .as_deref()
                        .and_then(|path| IconSource::of_path(package, path)),
                )
            })
            .collect();
        Arc::new(Self {
            manifest: package.manifest.clone(),
            revision: package.digest(),
            icon: IconSource::of(package),
            tab_icons,
            installed_at,
            origin: package.origin.clone(),
            lan_switched: webui::switched(package),
            lan_webui,
            firewalled,
            limits,
        })
    }

    pub fn of_record(package: &PluginPackage, record: &InstalledRecord) -> Arc<Self> {
        Self::new(
            package,
            record.installed_at,
            record.lan_webui,
            record.firewalled,
            record.limits,
        )
    }

    /// 开关是 `enabled` 时开着的局域网端口。
    pub fn open_ports(&self, enabled: bool) -> Vec<PortSpec> {
        webui::open_ports_of(&self.manifest, self.lan_switched.as_ref(), enabled)
    }

    #[cfg(test)]
    pub fn of_manifest(manifest: Manifest) -> Arc<Self> {
        Arc::new(Self {
            revision: manifest.version.clone(),
            manifest,
            icon: None,
            tab_icons: Vec::new(),
            installed_at: 0,
            origin: None,
            lan_switched: None,
            lan_webui: false,
            firewalled: false,
            limits: PluginLimits::default(),
        })
    }
}
