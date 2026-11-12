//! 安装的入口：测试和 MCP 总线、管理接口各自从这里进，走同一条 [`PluginManager::install_checked`]。

use std::sync::Arc;

use super::manager::{InstallError, PluginManager, PluginView, detached};
use super::manifest::{Permission, PluginKind};
use super::package::PluginPackage;

impl PluginManager {
    /// 安装只有 compose 文本的插件包，见 [`Self::install_package`]。
    pub async fn install(
        &self,
        source: &str,
        granted: &[Permission],
    ) -> Result<PluginView, InstallError> {
        self.install_package(PluginPackage::parse(source)?, granted)
            .await
    }

    /// 安装插件。`granted` 必须与清单声明的权限完全一致：这就是主人的确认。开关 `lan_webui` 关着。
    pub async fn install_package(
        &self,
        package: PluginPackage,
        granted: &[Permission],
    ) -> Result<PluginView, InstallError> {
        self.install_user(package, granted, false).await
    }

    /// `lan_webui`：主人看过风险后打开了开关。系统插件只能由安装器预装（见 [`super::system`]）。
    pub async fn install_user(
        &self,
        package: PluginPackage,
        granted: &[Permission],
        lan_webui: bool,
    ) -> Result<PluginView, InstallError> {
        if package.manifest.kind == PluginKind::System {
            return Err(InstallError::Unsupported(
                "system 插件只能由安装器预装".to_owned(),
            ));
        }
        self.install_checked(package, granted, lan_webui).await
    }

    /// 同 [`Self::install_user`]，但在独立的任务里执行（见 [`detached`]）：网关和 MCP 总线都经这里安装。
    pub async fn install_detached(
        self: &Arc<Self>,
        package: PluginPackage,
        granted: Vec<Permission>,
        lan_webui: bool,
    ) -> Result<PluginView, InstallError> {
        let this = Arc::clone(self);
        detached(async move { this.install_user(package, &granted, lan_webui).await }).await
    }
}
