//! 按来源预览和安装（插件规范的“来源”）。安装时按来源重新取一次插件包，摘要要和预览时的一样：主人确认的就是
//! 装上的；tag 在这之间被移动了，就得重新预览、重新确认。

use std::sync::Arc;

use wonton_protocol::{InstallPreview, InstallRequest};

use super::github_source::{self, SourceRef};
use super::languages::Languages;
use super::manager::{InstallError, PluginManager, PluginView};
use super::manifest::Permission;
use super::package::PluginPackage;

impl PluginManager {
    /// 按来源取插件包：先解析成提交，再按提交取文件。
    /// 是官方索引里的那个版本时，插件包按索引核对（信任级别是官方）。
    pub async fn fetch_source(&self, source: &str) -> Result<PluginPackage, InstallError> {
        let source = SourceRef::parse(source)?;
        let (index, _, _) = self.storefront.official(false).await;
        if let Some(listed) = index.find(&source).filter(|l| l.matches(&source)) {
            return self.storefront.files().package(listed).await;
        }
        let origin = github_source::resolve(&self.github, &source).await?;
        github_source::fetch(&self.github, &origin).await
    }

    pub async fn preview_source(
        &self,
        source: &str,
        langs: &Languages,
    ) -> Result<InstallPreview, InstallError> {
        let package = self.fetch_source(source).await?;
        self.preview_install(&package, langs).await
    }

    /// 按预览给的来源和摘要安装，`grant` 是主人确认的权限。安装本身在独立的任务里执行，见 [`Self::install_detached`]。
    pub async fn install_source(
        self: &Arc<Self>,
        request: &InstallRequest,
    ) -> Result<PluginView, InstallError> {
        let granted = request
            .grant
            .iter()
            .map(|p| p.parse::<Permission>())
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| InstallError::Invalid(e.to_string()))?;
        let package = self.fetch_source(&request.source).await?;
        let digest = package.digest();
        if digest != request.digest {
            return Err(InstallError::Changed(format!(
                "{} 在预览之后变了（插件包摘要现在是 {digest}），要重新预览、重新确认",
                request.source
            )));
        }
        self.install_detached(package, granted).await
    }
}
