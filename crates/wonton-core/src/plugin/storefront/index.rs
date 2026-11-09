//! 官方索引（插件规范的“官方索引”）：`index.json` 里每个插件一项，带着插件包每个文件的 sha256 和 compose 原文。
//! 商店列表直接用索引里的 compose，不逐个去取；安装时按 sha256 核对取到的每个文件，所以文件从哪里取都行：
//! 本地的索引目录（`WONTON_STORE_INDEX`，文件按 sha256 放在 `blobs/` 下），或者按提交从 GitHub 取。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use wonton_protocol::StoreUnreadable;

use super::super::github::{GitHub, Repo, is_commit_id};
use super::super::github_source::{Origin, SourceRef, Trust, Wanted, github_error};
use super::super::index_location::IndexLocation;
use super::super::install_error::InstallError;
use super::super::languages::Languages;
use super::super::manifest::Manifest;
use super::super::package::PluginPackage;
use super::super::package_files::{COMPOSE_FILE, MAX_FILE_BYTES, PackageFiles, sha256_hex};
use super::onboarding::OnboardingCatalog;

/// 核心读得懂的索引格式。
pub const FORMAT: u32 = 1;

/// `index.json`。
#[derive(Debug, Clone, Deserialize)]
struct IndexFile {
    format: u32,
    /// 第三方索引自己写的名字。
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    plugins: Vec<IndexEntry>,
    /// 读不懂时只影响帮助文档，所以这里不按 [`DocsSpec`] 解析。
    #[serde(default)]
    docs: Option<serde_json::Value>,
    /// 读不懂时只影响引导，所以这里不按 [`OnboardingCatalog`] 解析。
    #[serde(default)]
    onboarding: Option<serde_json::Value>,
    #[serde(default)]
    registry_mirrors: Option<serde_json::Value>,
}

/// 帮助文档的最新提交（ADR-0034）。
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct DocsSpec {
    /// `github:<owner>/<repo>`，不带版本。
    pub source: String,
    pub commit: String,
    /// 这份文档要的最低核心版本。
    pub min_core: String,
    /// 路径 → sha256 的十六进制。
    pub files: BTreeMap<String, String>,
}

/// 索引里的一个插件。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexEntry {
    /// `github:<owner>/<repo>`，不带版本。
    pub source: String,
    pub version: String,
    pub commit: String,
    /// 插件包的全部文件：路径 → sha256 的十六进制。
    pub files: BTreeMap<String, String>,
    /// `docker-compose.yml` 的原文。
    pub compose: String,
}

impl IndexEntry {
    /// 插件包在仓库 `source`（不带版本）的提交 `commit` 上的那一项：`plugin index-entry` 用它生成索引条目。核心不连
    /// Git，插件包要是那个提交的检出，由调用的人保证；装的时候按 sha256 核对，对不上就装不了。
    pub fn of(package: &PluginPackage, source: &str, commit: &str) -> Result<Self, String> {
        let entry = Self {
            source: source.to_owned(),
            version: package.manifest.version.clone(),
            commit: commit.to_owned(),
            files: Self::blobs(package)
                .map(|(path, data)| (path.to_owned(), sha256_hex(data)))
                .collect(),
            compose: package.source.clone(),
        };
        check(entry.clone(), &[], &Languages::default())?;
        Ok(entry)
    }

    /// 插件包的全部文件，含 compose：本地的索引目录按 sha256 把它们放在 `blobs/` 下。
    pub fn blobs(package: &PluginPackage) -> impl Iterator<Item = (&str, &[u8])> {
        let compose = (COMPOSE_FILE, package.source.as_bytes());
        let files = package.files().iter();
        std::iter::once(compose).chain(files.map(|(path, data)| (path.as_str(), data.as_slice())))
    }

    /// 把这一项写进本地的索引目录：文件放进 `blobs/`，`index.json` 里同一个来源的旧项换成这一项，别的项原样留着。
    /// 没有 `index.json` 时新建一份。
    pub fn write_to(&self, dir: &Path, package: &PluginPackage) -> Result<(), String> {
        let blobs = dir.join("blobs");
        std::fs::create_dir_all(&blobs)
            .map_err(|e| format!("cannot create {}: {e}", blobs.display()))?;
        for (_, data) in Self::blobs(package) {
            let path = blobs.join(sha256_hex(data));
            std::fs::write(&path, data)
                .map_err(|e| format!("cannot write {}: {e}", path.display()))?;
        }
        let path = dir.join("index.json");
        let mut index = match std::fs::read(&path) {
            Ok(json) => serde_json::from_slice(&json)
                .map_err(|e| format!("cannot parse {}: {e}", path.display()))?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                serde_json::json!({ "format": FORMAT, "plugins": [] })
            }
            Err(e) => return Err(format!("cannot read {}: {e}", path.display())),
        };
        let plugins = index
            .get_mut("plugins")
            .and_then(serde_json::Value::as_array_mut)
            .ok_or_else(|| format!("{} has no plugins list", path.display()))?;
        plugins
            .retain(|p| p.get("source").and_then(serde_json::Value::as_str) != Some(&self.source));
        plugins.push(serde_json::to_value(self).expect("索引项能写成 JSON"));
        let mut json = serde_json::to_vec_pretty(&index).expect("索引能写成 JSON");
        json.push(b'\n');
        std::fs::write(&path, json).map_err(|e| format!("cannot write {}: {e}", path.display()))
    }
}

/// 读懂、核对过的一项。
#[derive(Debug, Clone)]
pub struct Listed {
    pub entry: IndexEntry,
    pub repo: Repo,
    pub manifest: Manifest,
}

impl Listed {
    /// 装的就是索引里的这个版本。
    pub fn origin(&self) -> Origin {
        Origin {
            repo: self.entry.source.clone(),
            tag: Some(format!("v{}", self.entry.version)),
            commit: self.entry.commit.clone(),
            trust: Trust::Official,
            index: None,
        }
    }

    /// 来源要的是不是索引里的这个版本：不带版本、tag 是 `v<索引里的版本>`、或者就是索引里的提交。
    pub fn matches(&self, source: &SourceRef) -> bool {
        source.repo == self.repo
            && match &source.wanted {
                Wanted::Latest => true,
                Wanted::Tag(tag) => tag.strip_prefix('v') == Some(self.entry.version.as_str()),
                Wanted::Commit(commit) => commit == &self.entry.commit,
            }
    }
}

/// 读懂的索引：能用的项，和读不懂、对不上的项。
#[derive(Debug, Clone, Default)]
pub struct Index {
    pub name: Option<String>,
    pub plugins: Vec<Listed>,
    pub skipped: Vec<StoreUnreadable>,
    pub docs: Option<DocsSpec>,
    /// `docs` 读不懂的原因。
    pub docs_problem: Option<String>,
    pub onboarding: Option<OnboardingCatalog>,
    /// `onboarding` 读不懂的原因。
    pub onboarding_problem: Option<String>,
    pub registry_mirrors: Vec<String>,
    pub registry_mirrors_problem: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum IndexError {
    /// 索引的格式比核心新：要升级核心。消息不带语言（这里拿不到），给 App 的整句由 [`Self::message`] 按
    /// 请求的语言写（ADR-0043）。
    #[error("{0}")]
    NewerFormat(u32),
    #[error("{0}")]
    Invalid(String),
}

impl IndexError {
    /// 给 App 的整句，按语言写。
    pub(crate) fn message(&self, langs: &Languages) -> String {
        match self {
            Self::NewerFormat(format) => crate::l10n::text(
                langs,
                "plugin-settings-store-index-newer",
                &[
                    ("format", format.to_string().into()),
                    ("supported", FORMAT.to_string().into()),
                ],
            ),
            Self::Invalid(error) => crate::l10n::text(
                langs,
                "plugin-settings-store-index-invalid",
                &[("error", error.as_str().into())],
            ),
        }
    }
}

impl Index {
    pub fn parse(json: &[u8], langs: &Languages) -> Result<Self, IndexError> {
        let format = serde_json::from_slice::<serde_json::Value>(json)
            .map_err(|e| IndexError::Invalid(e.to_string()))?
            .get("format")
            .and_then(serde_json::Value::as_u64);
        match format {
            Some(f) if f != u64::from(FORMAT) => {
                return Err(IndexError::NewerFormat(
                    u32::try_from(f).unwrap_or(u32::MAX),
                ));
            }
            Some(_) => {}
            None => {
                let why = crate::l10n::text(langs, "plugin-settings-store-index-no-format", &[]);
                return Err(IndexError::Invalid(why));
            }
        }
        let file: IndexFile =
            serde_json::from_slice(json).map_err(|e| IndexError::Invalid(e.to_string()))?;
        debug_assert_eq!(file.format, FORMAT);
        let mut index = Self {
            name: file.name,
            ..Self::default()
        };
        match file.docs.map(serde_json::from_value::<DocsSpec>) {
            Some(Ok(docs)) => index.docs = Some(docs),
            Some(Err(e)) => {
                index.docs_problem = Some(crate::l10n::text(
                    langs,
                    "plugin-settings-store-index-docs-problem",
                    &[("error", e.to_string().into())],
                ))
            }
            None => {}
        }
        match file
            .onboarding
            .map(serde_json::from_value::<OnboardingCatalog>)
        {
            Some(Ok(onboarding)) => index.onboarding = Some(onboarding),
            Some(Err(e)) => {
                index.onboarding_problem = Some(crate::l10n::text(
                    langs,
                    "plugin-settings-store-index-onboarding-problem",
                    &[("error", e.to_string().into())],
                ))
            }
            None => {}
        }
        match file.registry_mirrors.map(|value| {
            // 索引在缓存刷新时解析，校验消息按默认语言（ADR-0043 的回退）。
            super::super::registry_mirror::address::choices(
                value,
                &super::super::languages::Languages::default(),
            )
        }) {
            Some(Ok(choices)) => index.registry_mirrors = choices,
            Some(Err(e)) => {
                let reason = crate::l10n::text(
                    langs,
                    "plugin-settings-store-index-mirrors-problem",
                    &[("error", e.to_string().into())],
                );
                tracing::warn!(%reason);
                index.registry_mirrors_problem = Some(reason);
            }
            None => {}
        }
        for entry in file.plugins {
            let source = entry.source.clone();
            match check(entry, &index.plugins, langs) {
                Ok(listed) => index.plugins.push(listed),
                Err(reason) => {
                    tracing::warn!(%source, %reason, "索引里的一项用不了，跳过");
                    index.skipped.push(StoreUnreadable { source, reason });
                }
            }
        }
        Ok(index)
    }

    pub fn find(&self, source: &SourceRef) -> Option<&Listed> {
        self.plugins.iter().find(|p| p.repo == source.repo)
    }
}

/// 一项要来源合法、提交是 40 位、compose 和 `files` 里记的一致、清单读得懂，来源和插件 ID 都不和前面的重复。
fn check(entry: IndexEntry, before: &[Listed], langs: &Languages) -> Result<Listed, String> {
    let text = |id: &str, args: &[(&str, wonton_l10n::Arg<'_>)]| crate::l10n::text(langs, id, args);
    let source = SourceRef::parse(&entry.source, langs).map_err(|e| e.to_string())?;
    if source.wanted != Wanted::Latest {
        return Err(text("plugin-settings-store-index-source-with-version", &[]));
    }
    if !is_commit_id(&entry.commit) {
        return Err(text("plugin-settings-store-index-commit-length", &[]));
    }
    if entry.files.get(COMPOSE_FILE) != Some(&sha256_hex(entry.compose.as_bytes())) {
        return Err(text(
            "plugin-settings-store-index-compose-digest",
            &[("file", COMPOSE_FILE.into())],
        ));
    }
    let manifest = PluginPackage::manifest_of(&entry.compose, langs).map_err(|e| e.to_string())?;
    if manifest.version != entry.version {
        return Err(text(
            "plugin-settings-store-index-version-mismatch",
            &[
                ("version", entry.version.as_str().into()),
                ("manifest", manifest.version.as_str().into()),
            ],
        ));
    }
    if before.iter().any(|p| p.repo == source.repo) {
        return Err(text("plugin-settings-store-index-source-duplicate", &[]));
    }
    if before.iter().any(|p| p.manifest.id == manifest.id) {
        return Err(text(
            "plugin-settings-store-index-id-duplicate",
            &[("id", manifest.id.to_string().into())],
        ));
    }
    Ok(Listed {
        entry,
        repo: source.repo,
        manifest,
    })
}

/// 索引和插件包的文件从哪里取。
#[derive(Debug, Clone)]
pub enum Files {
    /// 本地的索引目录：`index.json` 和 `blobs/<sha256>`。
    Dir(PathBuf),
    /// 按提交从 GitHub 取。
    GitHub(GitHub),
    /// 第三方索引：先取它旁边的 `blobs/<sha256>`，取不到再按提交从 GitHub 取（ADR-0031）。
    Third {
        location: IndexLocation,
        github: GitHub,
    },
}

impl Files {
    /// 插件包里的一个文件，按索引记的 sha256 核对。
    pub async fn file(
        &self,
        listed: &Listed,
        path: &str,
        langs: &Languages,
    ) -> Result<Vec<u8>, InstallError> {
        let sha256 = listed.entry.files.get(path).ok_or_else(|| {
            InstallError::Invalid(crate::l10n::text(
                langs,
                "plugin-settings-store-index-file-missing",
                &[
                    ("source", listed.entry.source.as_str().into()),
                    ("path", path.into()),
                ],
            ))
        })?;
        self.verified(&listed.repo, &listed.entry.commit, path, sha256, langs)
            .await
    }

    /// 仓库在某个提交里的一个文件，按索引记的 sha256 核对。
    pub async fn verified(
        &self,
        repo: &Repo,
        commit: &str,
        path: &str,
        sha256: &str,
        langs: &Languages,
    ) -> Result<Vec<u8>, InstallError> {
        let data = match self {
            Self::Dir(dir) => read_blob(dir, sha256, langs)?,
            Self::GitHub(github) => github
                .file(repo, commit, path, MAX_FILE_BYTES, langs)
                .await
                .map_err(github_error)?,
            Self::Third { location, github } => {
                let blob = format!("blobs/{sha256}");
                match location.read(github, &blob, MAX_FILE_BYTES, langs).await {
                    Ok(data) => data,
                    Err(_) => github
                        .file(repo, commit, path, MAX_FILE_BYTES, langs)
                        .await
                        .map_err(github_error)?,
                }
            }
        };
        if sha256_hex(&data) != sha256 {
            return Err(InstallError::Invalid(crate::l10n::text(
                langs,
                "plugin-settings-store-index-file-digest",
                &[("repo", repo.to_string().into()), ("path", path.into())],
            )));
        }
        Ok(data)
    }

    /// 索引里这一项的插件包：compose 用索引里的，引用的文件逐个取来核对。
    pub async fn package(
        &self,
        listed: &Listed,
        langs: &Languages,
    ) -> Result<PluginPackage, InstallError> {
        let compose = &listed.entry.compose;
        let mut files = PackageFiles::new();
        let optional = PluginPackage::optional_files(compose, langs)?;
        for path in PluginPackage::referenced_files(compose, langs)? {
            if optional.contains(&path) && !listed.entry.files.contains_key(&path) {
                continue;
            }
            let data = self.file(listed, &path, langs).await?;
            files.insert(path, data);
        }
        let mut package = PluginPackage::with_files(compose, files, langs)?;
        package.origin = Some(listed.origin());
        Ok(package)
    }
}

fn read_blob(dir: &Path, sha256: &str, langs: &Languages) -> Result<Vec<u8>, InstallError> {
    // sha256 来自索引：只收 64 位十六进制，拼进路径前先确认，免得 `../` 跑出目录。
    if sha256.len() != 64 || !sha256.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(InstallError::Invalid(crate::l10n::text(
            langs,
            "plugin-settings-store-index-blob-invalid-sha",
            &[("sha256", sha256.into())],
        )));
    }
    let path = dir.join("blobs").join(sha256);
    std::fs::read(&path).map_err(|e| {
        InstallError::NotFound(crate::l10n::text(
            langs,
            "plugin-settings-store-index-blob-missing",
            &[
                ("path", path.display().to_string().into()),
                ("error", e.to_string().into()),
            ],
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_onboarding_catalog() {
        let json = r#"{
            "format": 1,
            "plugins": [],
            "onboarding": {
                "agents": ["com.example.agent"],
                "needs": [{
                    "id": "photos",
                    "title": {"zh": "备份照片", "en": "Back up photos"},
                    "plugins": ["app.example.photos"]
                }],
                "providers": {
                    "en": [{"id": "openrouter", "key_url": "https://openrouter.ai/keys"}]
                },
                "more_providers": [{"id": "deepseek", "key_url": "https://platform.deepseek.com/api_keys"}],
                "prompts": [{"zh": "你能帮我做什么", "en": "What can you help me with"}]
            }
        }"#;

        let index = Index::parse(json.as_bytes(), &Languages::default()).unwrap();
        let onboarding = index.onboarding.unwrap();
        assert_eq!(onboarding.agents, ["com.example.agent"]);
        assert_eq!(onboarding.needs[0].plugins, ["app.example.photos"]);
        assert_eq!(
            onboarding.providers["en"][0].key_url,
            "https://openrouter.ai/keys"
        );
        assert_eq!(onboarding.more_providers[0].id, "deepseek");
        assert_eq!(
            onboarding.more_providers[0].key_url,
            "https://platform.deepseek.com/api_keys"
        );
        assert_eq!(index.onboarding_problem, None);
    }

    #[test]
    fn unknown_onboarding_fields_are_ignored() {
        let json = r#"{
            "format": 1,
            "plugins": [],
            "onboarding": {
                "agents": ["com.example.agent"],
                "needs": [{
                    "id": "photos",
                    "title": {"en": "Photos"},
                    "plugins": ["app.example.photos"],
                    "future_need": true
                }],
                "providers": {
                    "en": [{
                        "id": "openrouter",
                        "key_url": "https://openrouter.ai/keys",
                        "future_provider": true
                    }]
                },
                "more_providers": [],
                "prompts": [{"en": "What can you help me with"}],
                "future_catalog": true
            }
        }"#;

        let index = Index::parse(json.as_bytes(), &Languages::default()).unwrap();
        let onboarding = index.onboarding.unwrap();
        assert_eq!(onboarding.needs[0].id, "photos");
        assert_eq!(onboarding.providers["en"][0].id, "openrouter");
        assert_eq!(index.onboarding_problem, None);
    }

    #[test]
    fn an_unreadable_onboarding_catalog_does_not_reject_the_index() {
        let json = r#"{
            "format": 1,
            "plugins": [],
            "onboarding": {"agents": "not a list"}
        }"#;

        let index = Index::parse(json.as_bytes(), &Languages::default()).unwrap();
        assert_eq!(index.onboarding, None);
        assert!(index.onboarding_problem.is_some());
    }

    #[test]
    fn missing_more_providers_rejects_only_the_onboarding_catalog() {
        let json = br#"{
            "format": 1,
            "plugins": [],
            "onboarding": {"agents": [], "needs": [], "providers": {}, "prompts": []}
        }"#;

        let index = Index::parse(json, &Languages::default()).unwrap();
        assert_eq!(index.onboarding, None);
        assert!(index.onboarding_problem.is_some());
    }

    #[test]
    fn an_index_without_onboarding_has_no_catalog_problem() {
        let index =
            Index::parse(br#"{"format": 1, "plugins": []}"#, &Languages::default()).unwrap();

        assert_eq!(index.onboarding, None);
        assert_eq!(index.onboarding_problem, None);
    }
}
