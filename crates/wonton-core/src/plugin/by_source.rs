//! 按来源预览和安装（插件规范的“来源”）。安装时按来源重新取一次插件包，摘要要和预览时的一样：主人确认的就是
//! 装上的；tag 在这之间被移动了，就得重新预览、重新确认。

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use wonton_protocol::{AddedSource, InstallPreview, InstallRequest, StoreProblem};

use super::github_source::{self, INDEX_SCHEME, SourceRef};
use super::index_location::IndexLocation;
use super::languages::{Languages, accept_language_tags};
use super::manager::{InstallError, PluginManager, PluginView};
use super::package::PluginPackage;
use super::staged;
use super::store::{create_private_dir, read_json, read_people_languages, write_json};
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

    /// 记下 `username` 最近一次请求的语言（ADR-0043）：语言按人给，主人和家人各记各的。只在变了时写盘，
    /// 写不了时下次再试。头里没有能用的语言（空、只有 `*`）时不写，免得把好的记录盖掉。
    pub fn note_person_languages(&self, username: &str, header: Option<&str>) {
        let Some(header) = header else { return };
        if accept_language_tags(header).is_empty() {
            return;
        }
        let _writing = self.people_writes.lock().expect("语言记录锁中毒");
        let path = self.store.people_languages();
        // 只有文件不在算空表；读不了或坏了时整份表会被这条盖掉，所以这次不写，下次再试。
        let mut people = match read_json::<BTreeMap<String, String>>(&path) {
            Ok(Some(people)) => people,
            Ok(None) => BTreeMap::new(),
            Err(error) => {
                tracing::warn!(%error, path = %path.display(), person = %username, "每个人的语言读不出来，这次不写");
                return;
            }
        };
        if people.get(username).map(String::as_str) == Some(header) {
            return;
        }
        people.insert(username.to_owned(), header.to_owned());
        let written = path
            .parent()
            .map_or(Ok(()), create_private_dir)
            .and_then(|()| write_json(&path, &people));
        if let Err(error) = written {
            tracing::warn!(%error, person = %username, "这个人的语言存不下");
        }
    }

    /// 某人最近一次请求的 `Accept-Language` 原文；没有记录时为空。
    pub fn person_languages(&self, username: &str) -> Option<String> {
        self.stored_languages().get(username).cloned()
    }

    /// 某人最近一次请求的第一个语言标签，原样（OIDC 的 `locale`）；没有记录时为空。
    pub fn person_locale(&self, username: &str) -> Option<String> {
        let header = self.person_languages(username)?;
        accept_language_tags(&header).into_iter().next()
    }

    /// 主人的语言：请求里没有 `Accept-Language` 的路径（Agent 经 MCP 调用）按它写。
    pub fn owner_languages(&self) -> Languages {
        Languages::from_header(self.person_languages(&self.cfg.owner).as_deref())
    }

    /// 整份表；读不了时按没有记录，但打一条日志，不静默。
    fn stored_languages(&self) -> BTreeMap<String, String> {
        read_people_languages(&self.store.people_languages())
    }

    /// 主人的用户名。
    pub fn owner_name(&self) -> &str {
        &self.cfg.owner
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

    /// 按预览给的来源和摘要安装，`grant` 是主人确认的权限、`enabled_permissions` 是打开了的可选项。安装本身在独立的
    /// 任务里执行，见 [`Self::install_detached`]。
    pub async fn install_source(
        self: &Arc<Self>,
        request: &InstallRequest,
    ) -> Result<PluginView, InstallError> {
        let granted = super::store::parse_permissions(&request.grant)
            .map_err(|e| InstallError::Invalid(e.to_string()))?;
        let enabled = super::store::parse_enabled(
            &request.enabled_permissions,
            super::store::OpenedBy::Owner,
            super::manager::now_secs(),
        )
        .map_err(|e| InstallError::Invalid(e.to_string()))?;
        let package = self.fetch_source(&request.source).await?;
        let digest = package.digest();
        if digest != request.digest {
            return Err(InstallError::Changed(format!(
                "{} 在预览之后变了（插件包摘要现在是 {digest}），要重新预览、重新确认",
                request.source
            )));
        }
        let langs = self.owner_languages();
        self.store_input_values(&package.manifest, &request.secrets, None, &langs)?;
        let settings = super::settings::install_values(&package, &request.settings, &langs)?;
        let installed = self
            .install_detached_enabled(
                package,
                granted,
                enabled,
                request.lan_webui,
                settings,
                langs,
            )
            .await?;
        self.staged.remove(&digest);
        Ok(installed)
    }
}
