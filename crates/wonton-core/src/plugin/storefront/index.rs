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
use super::super::registry_mirror::mirrors::Mirrors;
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
    /// `github:<owner>/<repo>` 或 `github:<owner>/<repo>/<子目录>`，不带版本。
    pub source: String,
    pub version: String,
    pub commit: String,
    /// 插件包的全部文件：路径 → sha256 的十六进制。
    pub files: BTreeMap<String, String>,
    /// `docker-compose.yml` 的原文。
    pub compose: String,
    /// 清单里的 `license`，给不读 compose 的索引使用方；写了就和清单核对，对不上时这一项读不懂。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,
    /// 清单里的 `license_url`。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license_url: Option<String>,
}

impl IndexEntry {
    /// 插件包在仓库 `source`（不带版本，可以带子目录）的提交 `commit` 上的那一项：`plugin index-entry` 用它生成索引
    /// 条目。核心不连 Git，插件包要是那个提交（子目录）的检出，由调用的人保证；装的时候按 sha256 核对，对不上就装不了。
    pub fn of(package: &PluginPackage, source: &str, commit: &str) -> Result<Self, String> {
        let entry = Self {
            source: source.to_owned(),
            version: package.manifest.version.clone(),
            commit: commit.to_owned(),
            files: Self::blobs(package)
                .map(|(path, data)| (path, sha256_hex(data)))
                .collect(),
            compose: package.source.clone(),
            license: package.manifest.license.clone(),
            license_url: package.manifest.license_url.clone(),
        };
        check(entry.clone(), &[], &Languages::default())?;
        Ok(entry)
    }

    /// 插件包的全部文件，含 compose：本地的索引目录按 sha256 把它们放在 `blobs/` 下。
    pub fn blobs(package: &PluginPackage) -> impl Iterator<Item = (String, &[u8])> {
        let compose = (COMPOSE_FILE, package.source.as_bytes());
        let files = package
            .files()
            .iter()
            .map(|(path, data)| (path.clone(), data.as_slice()));
        std::iter::once((COMPOSE_FILE.to_owned(), compose.1))
            .chain(files)
            .chain(package.static_blobs())
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
    /// 插件包在仓库里的子目录；根目录时为空。
    pub subdir: Option<String>,
    pub manifest: Manifest,
}

impl Listed {
    /// 给主人看的提供方，如 `github.com/owner/repo`：子目录不影响。
    pub fn provider(&self) -> String {
        format!("github.com/{}", self.repo)
    }

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
            && source.subdir == self.subdir
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
    pub registry_mirrors: Mirrors,
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
            super::super::registry_mirror::mirrors::parse(
                value,
                &super::super::languages::Languages::default(),
            )
        }) {
            Some(Ok(mirrors)) => index.registry_mirrors = mirrors,
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
        self.plugins
            .iter()
            .find(|p| p.repo == source.repo && p.subdir == source.subdir)
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
    // 旧索引没有这几项，读得懂；写了就要和清单一致，对不上说明索引过时或被手改过。
    for (field, listed, written) in [
        ("license", &entry.license, &manifest.license),
        ("license_url", &entry.license_url, &manifest.license_url),
    ] {
        if listed
            .as_ref()
            .is_some_and(|listed| Some(listed) != written.as_ref())
        {
            return Err(text(
                "plugin-settings-store-index-license-mismatch",
                &[
                    ("field", field.into()),
                    ("index", listed.as_deref().unwrap_or("").into()),
                    ("manifest", written.as_deref().unwrap_or("").into()),
                ],
            ));
        }
    }
    if manifest.version != entry.version {
        return Err(text(
            "plugin-settings-store-index-version-mismatch",
            &[
                ("version", entry.version.as_str().into()),
                ("manifest", manifest.version.as_str().into()),
            ],
        ));
    }
    if before
        .iter()
        .any(|p| p.repo == source.repo && p.subdir == source.subdir)
    {
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
        subdir: source.subdir,
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
        self.verified(
            &listed.repo,
            listed.subdir.as_deref(),
            &listed.entry.commit,
            path,
            sha256,
            langs,
        )
        .await
    }

    /// 仓库在某个提交里的一个文件，按索引记的 sha256 核对。
    pub async fn verified(
        &self,
        repo: &Repo,
        subdir: Option<&str>,
        commit: &str,
        path: &str,
        sha256: &str,
        langs: &Languages,
    ) -> Result<Vec<u8>, InstallError> {
        self.verified_with(repo, subdir, commit, path, sha256, MAX_FILE_BYTES, langs)
            .await
    }

    /// 同 [`Self::verified`]，单个文件的上限由调用方给（静态网页的文件比引用文件大，ADR-0080）。
    #[allow(clippy::too_many_arguments)]
    pub async fn verified_with(
        &self,
        repo: &Repo,
        subdir: Option<&str>,
        commit: &str,
        path: &str,
        sha256: &str,
        max: u64,
        langs: &Languages,
    ) -> Result<Vec<u8>, InstallError> {
        let full = subdir.map_or_else(|| path.to_owned(), |subdir| format!("{subdir}/{path}"));
        let data = match self {
            Self::Dir(dir) => read_blob(dir, sha256, langs)?,
            Self::GitHub(github) => github
                .file(repo, commit, &full, max, langs)
                .await
                .map_err(github_error)?,
            Self::Third { location, github } => {
                let blob = format!("blobs/{sha256}");
                match location.read(github, &blob, max, langs).await {
                    Ok(data) => data,
                    Err(_) => github
                        .file(repo, commit, &full, max, langs)
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
        if let Some(root) = package.static_root() {
            let prefix = format!("{root}/");
            let mut found = super::super::package::static_files::StaticFiles::new();
            for (path, sha256) in &listed.entry.files {
                let Some(rel) = path.strip_prefix(&prefix) else {
                    continue;
                };
                let data = self
                    .verified_with(
                        &listed.repo,
                        listed.subdir.as_deref(),
                        &listed.entry.commit,
                        path,
                        sha256,
                        super::super::package::static_files::MAX_FILE_BYTES,
                        langs,
                    )
                    .await?;
                found.insert(rel.to_owned(), data);
            }
            package = package.with_static(found, langs)?;
        }
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
                "prompts": [{"zh": "你能帮我做什么", "en": "What can you help me with"}]
            }
        }"#;

        let index = Index::parse(json.as_bytes(), &Languages::default()).unwrap();
        let onboarding = index.onboarding.unwrap();
        assert_eq!(onboarding.agents, ["com.example.agent"]);
        assert_eq!(onboarding.needs[0].plugins, ["app.example.photos"]);
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
                "prompts": [{"en": "What can you help me with"}],
                "future_catalog": true
            }
        }"#;

        let index = Index::parse(json.as_bytes(), &Languages::default()).unwrap();
        let onboarding = index.onboarding.unwrap();
        assert_eq!(onboarding.needs[0].id, "photos");
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
    fn missing_needs_rejects_only_the_onboarding_catalog() {
        let json = br#"{
            "format": 1,
            "plugins": [],
            "onboarding": {"agents": [], "prompts": []}
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

    fn compose(id: &str) -> String {
        format!(
            "services:\n  web:\n    image: ghcr.io/acme/web:1\n\
             x-wonton:\n  spec: \"0.2\"\n  id: {id}\n  version: 1.0.0\n  kind: app\n  \
             title: {{zh: 你好, en: Hello}}\n  description: {{zh: 打个招呼, en: Say hi}}\n  permissions: []\n"
        )
    }

    fn entry(id: &str, source: &str) -> IndexEntry {
        let package =
            PluginPackage::parse(&compose(id), &Languages::default()).expect("compose 合法");
        IndexEntry::of(&package, source, &"a".repeat(40)).expect("条目合法")
    }

    /// 子目录来源的条目能读懂，且同一仓库的另一个子目录不算重复来源（完成标准 6 的一部分）。
    #[test]
    fn subdirectory_entries_are_understood_and_distinct_per_subdirectory() {
        let index = Index::parse(
            serde_json::to_vec(&serde_json::json!({
                "format": 1,
                "plugins": [
                    entry("app.codex", "github:acme/hello/agents/codex"),
                    entry("app.kimi", "github:acme/hello/apps/kimi"),
                ],
            }))
            .unwrap()
            .as_slice(),
            &Languages::default(),
        )
        .map_err(|e| panic!("{e}"))
        .unwrap();
        assert_eq!(index.plugins.len(), 2);
        assert_eq!(index.plugins[0].subdir.as_deref(), Some("agents/codex"));
        assert_eq!(index.plugins[1].subdir.as_deref(), Some("apps/kimi"));

        // 同一个来源两次才是重复；同一个插件 ID 两次也是重复。
        let duplicate = Index::parse(
            serde_json::to_vec(&serde_json::json!({
                "format": 1,
                "plugins": [
                    entry("app.codex", "github:acme/hello/agents/codex"),
                    entry("app.codex", "github:acme/hello/agents/codex"),
                ],
            }))
            .unwrap()
            .as_slice(),
            &Languages::default(),
        )
        .unwrap();
        assert_eq!(duplicate.plugins.len(), 1);
        assert_eq!(duplicate.skipped.len(), 1);
    }

    /// 读不懂的来源只跳过这一项，别的照常（老核心碰到子目录来源也是这样，完成标准 6 的一部分）。
    #[test]
    fn an_unreadable_source_is_skipped_but_the_rest_survives() {
        let index = Index::parse(
            serde_json::to_vec(&serde_json::json!({
                "format": 1,
                "plugins": [
                    {"source": "github:acme", "version": "1.0.0", "commit": "a".repeat(40), "files": {}, "compose": ""},
                    entry("app.codex", "github:acme/hello/agents/codex"),
                ],
            }))
            .unwrap()
            .as_slice(),
            &Languages::default(),
        )
        .unwrap();
        assert_eq!(index.plugins.len(), 1);
        let [skipped] = &index.skipped[..] else {
            panic!("{index:?}");
        };
        assert_eq!(skipped.source, "github:acme");
    }
}
