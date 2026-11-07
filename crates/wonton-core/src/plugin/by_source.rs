//! 按来源预览和安装（插件规范的“来源”）。安装时按来源重新取一次插件包，摘要要和预览时的一样：主人确认的就是
//! 装上的；tag 在这之间被移动了，就得重新预览、重新确认。

use std::path::PathBuf;
use std::sync::Arc;

use wonton_protocol::{AddedSource, InstallPreview, InstallRequest, StoreProblem};

use super::github_source::{self, INDEX_SCHEME, SourceRef};
use super::index_location::IndexLocation;
use super::languages::Languages;
use super::manager::{InstallError, PluginManager, PluginView};
use super::manifest::Permission;
use super::package::PluginPackage;
use super::staged;
use super::store::{create_private_dir, read_json, write_json};
use super::storefront::index::{Files, Index, Listed};

impl PluginManager {
    /// 按来源取插件包：先解析成提交，再按提交取文件。
    /// 是官方索引里的那个版本时，插件包按索引核对（信任级别是官方）。
    pub async fn fetch_source(&self, source: &str) -> Result<PluginPackage, InstallError> {
        let langs = self.owner_languages();
        if let Some(digest) = source.strip_prefix(staged::SCHEME) {
            return self.staged.get(digest).ok_or_else(|| {
                InstallError::NotFound("上传的文件已经过期，要重新选一次文件".to_owned())
            });
        }
        if source.starts_with(INDEX_SCHEME) {
            let (location, id) = IndexLocation::of_item_source(source, &langs)?;
            return self
                .storefront
                .added
                .package_in(&location, id, &langs)
                .await;
        }
        let source = SourceRef::parse(source, &langs)?;
        let (index, _, _) = self.storefront.official(false, &langs).await;
        if let Some(listed) = index.find(&source).filter(|l| l.matches(&source)) {
            return self.storefront.files().package(listed, &langs).await;
        }
        let origin = github_source::resolve(&self.github, &source, &langs).await?;
        github_source::fetch(&self.github, &origin, &langs).await
    }

    /// 添加来源（ADR-0031），能读到才写下来。
    pub async fn add_store_source(
        &self,
        address: &str,
        langs: &Languages,
    ) -> Result<AddedSource, InstallError> {
        let (index, _, _) = self.storefront.official(false, langs).await;
        let added = self.storefront.added.add(address, &index, langs).await?;
        Ok(self.added_section(&added, langs).await)
    }

    /// 删除添加的来源，已安装的插件不动。`address` 是 [`AddedSource::address`]。
    pub fn remove_store_source(
        &self,
        address: &str,
        langs: &Languages,
    ) -> Result<(), InstallError> {
        self.storefront.added.remove(address, langs)
    }

    /// 官方索引，和读它时遇到的问题：Bot 经 MCP 总线只能找、只能装这里的插件（ADR-0016）。
    pub async fn official_index(&self) -> (Index, Option<StoreProblem>) {
        let langs = self.owner_languages();
        let (index, _, problem) = self.storefront.official(false, &langs).await;
        (index, problem)
    }

    /// 主人主动操作（升级预览、手动刷新）时用的官方索引：不等缓存过期，离上次去取不到一分钟的除外。
    pub(in crate::plugin) async fn refreshed_official_index(
        &self,
        refresh: bool,
        langs: &Languages,
    ) -> (Index, Option<StoreProblem>) {
        let (index, _, problem) = self.storefront.official(refresh, langs).await;
        (index, problem)
    }

    /// 官方索引的文件从哪里取。
    pub fn official_files(&self) -> Files {
        self.storefront.files()
    }

    /// 帮助文档的缓存目录（ADR-0034）。
    pub fn docs_cache(&self) -> PathBuf {
        self.store.docs_cache()
    }

    /// 记下主人的语言：帮助文档的工具经 MCP 调用，请求里没有 `Accept-Language`。只在变了时写盘，写不了时下次再试。
    pub fn note_owner_languages(&self, header: Option<&str>) {
        let Some(header) = header else { return };
        if self.stored_languages().as_deref() == Some(header) {
            return;
        }
        let path = self.store.owner_languages();
        let written = path
            .parent()
            .map_or(Ok(()), create_private_dir)
            .and_then(|()| write_json(&path, &header));
        if let Err(error) = written {
            tracing::warn!(%error, "主人的语言存不下");
        }
    }

    pub fn owner_languages(&self) -> Languages {
        Languages::from_header(self.stored_languages().as_deref())
    }

    fn stored_languages(&self) -> Option<String> {
        read_json(&self.store.owner_languages()).ok().flatten()
    }

    /// 官方索引里一项的插件包，文件按索引核对。
    pub async fn official_package(&self, listed: &Listed) -> Result<PluginPackage, InstallError> {
        self.storefront
            .files()
            .package(listed, &self.owner_languages())
            .await
    }

    pub async fn preview_source(
        &self,
        source: &str,
        langs: &Languages,
    ) -> Result<InstallPreview, InstallError> {
        let package = self.fetch_source(source).await?;
        self.preview_install(&package, langs).await
    }

    /// 预览上传的插件包，并暂存到主人确认：预览的 `source` 就是它的来源。
    pub async fn preview_upload(
        &self,
        package: PluginPackage,
        langs: &Languages,
    ) -> Result<InstallPreview, InstallError> {
        let mut preview = self.preview_install(&package, langs).await?;
        preview.source = Some(self.staged.stage(package));
        Ok(preview)
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
        let installed = self
            .install_detached(package, granted, request.lan_webui, self.owner_languages())
            .await?;
        self.staged.remove(&digest);
        Ok(installed)
    }
}
