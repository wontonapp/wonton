//! 安装的入口：测试和 MCP 总线、管理接口各自从这里进，走同一条 [`PluginManager::install_checked`]。

use std::sync::Arc;

use super::languages::Languages;
use super::manager::{InstallError, PluginManager, PluginView, detached};
use super::manifest::{Permission, PluginKind};
use super::package::PluginPackage;
use super::store::EnabledPermission;
use crate::l10n;

impl PluginManager {
    /// 安装只有 compose 文本的插件包，见 [`Self::install_package`]。
    pub async fn install(
        &self,
        source: &str,
        granted: &[Permission],
        langs: Languages,
    ) -> Result<PluginView, InstallError> {
        self.install_package(PluginPackage::parse(source, &langs)?, granted, langs)
            .await
    }

    /// 安装插件。`granted` 必须与清单声明的权限完全一致：这就是主人的确认。开关 `lan_webui` 关着。
    pub async fn install_package(
        &self,
        package: PluginPackage,
        granted: &[Permission],
        langs: Languages,
    ) -> Result<PluginView, InstallError> {
        self.install_user(package, granted, false, langs).await
    }

    /// `lan_webui`：主人看过风险后打开了开关。系统插件只能由安装器预装（见 [`super::system`]）。
    pub async fn install_user(
        &self,
        package: PluginPackage,
        granted: &[Permission],
        lan_webui: bool,
        langs: Languages,
    ) -> Result<PluginView, InstallError> {
        self.install_user_enabled(package, granted, &[], lan_webui, langs)
            .await
    }

    /// 同 [`Self::install_user`]，另外带上主人打开了的可选权限（ADR-0057）。
    pub async fn install_user_enabled(
        &self,
        package: PluginPackage,
        granted: &[Permission],
        enabled: &[EnabledPermission],
        lan_webui: bool,
        langs: Languages,
    ) -> Result<PluginView, InstallError> {
        if package.manifest.kind == PluginKind::System {
            return Err(InstallError::Unsupported(l10n::text(
                &langs,
                "install-system-preinstalled",
                &[],
            )));
        }
        self.install_checked(package, granted, enabled, lan_webui, &langs)
            .await
    }

    /// 同 [`Self::install_user`]，但在独立的任务里执行（见 [`detached`]）：网关和 MCP 总线都经这里安装。
    pub async fn install_detached(
        self: &Arc<Self>,
        package: PluginPackage,
        granted: Vec<Permission>,
        lan_webui: bool,
        langs: Languages,
    ) -> Result<PluginView, InstallError> {
        let this = Arc::clone(self);
        detached(async move { this.install_user(package, &granted, lan_webui, langs).await }).await
    }

    /// 同 [`Self::install_detached`]，另外带上主人打开了的可选权限（ADR-0057）。
    pub async fn install_detached_enabled(
        self: &Arc<Self>,
        package: PluginPackage,
        granted: Vec<Permission>,
        enabled: Vec<EnabledPermission>,
        lan_webui: bool,
        langs: Languages,
    ) -> Result<PluginView, InstallError> {
        let this = Arc::clone(self);
        detached(async move {
            this.install_user_enabled(package, &granted, &enabled, lan_webui, langs)
                .await
        })
        .await
    }
}
