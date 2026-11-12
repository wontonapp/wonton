//! 给 [`PluginManager`] 接上可选的部分。

use std::sync::Arc;

use tokio::sync::watch;

use super::egress::core_proxy::CoreProxy;
use super::egress::settings::Egress;
#[cfg(test)]
use super::github::GitHub;
use super::gpu::HostPaths;
use super::lan::Lan;
use super::languages::Languages;
use super::manager::{InstallError, PluginManager};
use super::storefront::{StoreConfig, Storefront};
use crate::l10n;
use crate::matrix::Matrix;

impl PluginManager {
    /// 从测试替身取按来源安装的插件包，商店也用它。
    #[cfg(test)]
    #[must_use]
    pub fn with_github(mut self, github: GitHub) -> Self {
        self.storefront = Storefront::new(
            github.clone(),
            self.storefront.config().clone(),
            self.store.index_cache(),
        );
        self.github = github;
        self
    }

    /// 商店的配置：官方索引从哪里取，列不列社区插件。
    #[must_use]
    pub fn with_store(mut self, config: StoreConfig) -> Self {
        self.storefront = Storefront::new(self.github.clone(), config, self.store.index_cache());
        self
    }

    /// 接上 Matrix 服务器：声明了 `matrix.bot` 的插件这才装得上。
    #[must_use]
    pub fn with_matrix(mut self, matrix: Option<Arc<Matrix>>) -> Self {
        self.matrix = matrix;
        self
    }

    /// 核心看到的宿主路径（`WONTON_HOST_DEV`），申请了 `devices.gpu` 的插件按它探测显卡。
    #[must_use]
    pub fn with_host(mut self, host: HostPaths) -> Self {
        self.host = host;
        self
    }
    /// 测试用：设置插件容器的父 cgroup。
    #[cfg(test)]
    #[must_use]
    pub fn with_plugin_slice(mut self, slice: Option<String>) -> Self {
        self.cfg.plugin_slice = slice;
        self
    }

    /// 怎么检测局域网地址、`WONTON_LAN_ADDRESSES`（见 [`super::lan`]）。
    #[must_use]
    pub fn with_lan(mut self, lan: Lan) -> Self {
        self.lan = lan;
        self
    }

    /// `WONTON_EGRESS_PROXY` 和安装脚本检测到的代理（见 [`super::egress`]）。
    #[must_use]
    pub fn with_egress(mut self, egress: Egress) -> Self {
        self.egress = egress;
        self.follow_core_proxy();
        self
    }

    /// 核心自己访问外网用的代理，跟着设置走。
    pub fn core_proxy(&self) -> &CoreProxy {
        &self.core_proxy
    }

    /// 核心升级没结束时不装、不卸（ADR-0025）。接口和 Bot 经总线发起的都走到这里。
    #[must_use]
    pub fn with_upgrade_freeze(mut self, frozen: watch::Receiver<bool>) -> Self {
        self.upgrade_frozen = Some(frozen);
        self
    }

    pub(super) fn check_not_upgrading(&self, langs: &Languages) -> Result<(), InstallError> {
        match &self.upgrade_frozen {
            Some(frozen) if *frozen.borrow() => Err(InstallError::CoreUpgrading(l10n::text(
                langs,
                "plugin-settings-core-upgrading",
                &[],
            ))),
            _ => Ok(()),
        }
    }
}
