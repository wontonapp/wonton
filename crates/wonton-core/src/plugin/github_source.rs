//! 按来源取插件包（插件规范的“来源”）：`github:<owner>/<repo>[/<子目录>]`，可以带 `@<tag>` 或 `@<40 位提交号>`。先把来源
//! 解析成提交，再按提交取 `docker-compose.yml` 和它引用的文件，和上传的插件包走同一套解析和校验。插件包可以在仓库根目录，
//! 也可以在子目录里：文件路径相对插件包，摘要不随位置变化。

use serde::{Deserialize, Serialize};

use super::github::{GitHub, GitHubError, Repo, Tag, is_commit_id};
use super::index_location::IndexLocation;
use super::install_error::InstallError;
use super::languages::Languages;
use super::package::PluginPackage;
use super::package_files::{COMPOSE_FILE, MAX_FILE_BYTES, PackageFiles};
use crate::l10n;

const SCHEME: &str = "github:";
/// 第三方索引里的插件的来源的开头（ADR-0031）。
pub const INDEX_SCHEME: &str = "index+";

/// 来源里写的版本。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Wanted {
    /// 最新的正式版本：版本号最大的、不带预发布后缀的 `v<语义化版本>` tag。
    Latest,
    Tag(String),
    Commit(String),
}

/// 主人或 App 给的来源。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceRef {
    pub repo: Repo,
    /// 仓库里的插件包目录，如 `agents/kimi-code`；插件包在根目录时为空。
    pub subdir: Option<String>,
    pub wanted: Wanted,
}

impl SourceRef {
    pub fn parse(text: &str, langs: &Languages) -> Result<Self, InstallError> {
        let invalid = |why: &str| {
            InstallError::Invalid(crate::l10n::text(
                langs,
                "plugin-settings-github-source-invalid",
                &[("text", text.into()), ("why", why.into())],
            ))
        };
        let rest = text.strip_prefix(SCHEME).ok_or_else(|| {
            invalid(&crate::l10n::text(
                langs,
                "plugin-settings-github-source-scheme",
                &[],
            ))
        })?;
        let (path, version) = match rest.split_once('@') {
            Some((path, version)) => (path, Some(version)),
            None => (rest, None),
        };
        let (owner, name) = path.split_once('/').ok_or_else(|| {
            invalid(&crate::l10n::text(
                langs,
                "plugin-settings-github-source-no-name",
                &[],
            ))
        })?;
        let (name, subdir) = match name.split_once('/') {
            Some((name, subdir)) => (name, Some(subdir.to_owned())),
            None => (name, None),
        };
        if !is_name(owner) || !is_name(name) {
            return Err(invalid(&crate::l10n::text(
                langs,
                "plugin-settings-github-source-name-chars",
                &[],
            )));
        }
        if let Some(subdir) = &subdir {
            check_subdir(subdir, langs).map_err(|why| invalid(&why))?;
        }
        let wanted = match version {
            None => Wanted::Latest,
            Some(v) if is_commit_id(v) => Wanted::Commit(v.to_owned()),
            Some(v) if is_tag(v) => Wanted::Tag(v.to_owned()),
            Some(_) => {
                return Err(invalid(&crate::l10n::text(
                    langs,
                    "plugin-settings-github-source-bad-tag",
                    &[],
                )));
            }
        };
        let repo = Repo {
            owner: owner.to_owned(),
            name: name.to_owned(),
        };
        Ok(Self {
            repo,
            subdir,
            wanted,
        })
    }
}

impl SourceRef {
    /// 来源字符串，[`Self::parse`] 能读回来。
    pub fn text(&self) -> String {
        let source = self.without_version();
        match &self.wanted {
            Wanted::Latest => source,
            Wanted::Tag(version) | Wanted::Commit(version) => format!("{source}@{version}"),
        }
    }

    /// 不带版本的来源，如 `github:owner/repo/agents/codex`。
    pub fn without_version(&self) -> String {
        match &self.subdir {
            Some(subdir) => format!("{SCHEME}{}/{subdir}", self.repo),
            None => format!("{SCHEME}{}", self.repo),
        }
    }

    /// 子目录对应的 tag 前缀：取最后一段（`agents/kimi-code` → `kimi-code`）。同一仓库里最后一段不能重复。
    pub fn tag_prefix(&self) -> Option<&str> {
        self.subdir.as_deref()?.rsplit('/').next()
    }
}

/// 仓库名和 owner：GitHub 只允许这些字符；不能是 `.`、`..`，免得拼进地址后变成别的路径。
fn is_name(text: &str) -> bool {
    !text.is_empty()
        && text.len() <= 100
        && text != "."
        && text != ".."
        && text
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}

/// 子目录总长上限。每段只允许仓库名允许的字符；空段、`.`、`..` 都拒绝，免得拼进地址后变成别的路径。
fn check_subdir(text: &str, langs: &Languages) -> Result<(), String> {
    let key = if text.is_empty() || text.split('/').any(str::is_empty) {
        "plugin-settings-github-source-subdir-empty"
    } else if text.len() > 200 {
        "plugin-settings-github-source-subdir-long"
    } else if text.split('/').any(|part| matches!(part, "." | "..")) {
        "plugin-settings-github-source-subdir-dot"
    } else if !text
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'/'))
    {
        "plugin-settings-github-source-subdir-chars"
    } else {
        return Ok(());
    };
    Err(crate::l10n::text(langs, key, &[]))
}

/// tag 名：git 允许的字符很多，这里只收发布用得到的。子目录的 tag 带 `/`（如 `kimi-code/v0.1.0`），不能有空段。
fn is_tag(text: &str) -> bool {
    !text.is_empty()
        && text.len() <= 128
        && !text.starts_with(['.', '-'])
        && !text.starts_with('/')
        && !text.ends_with('/')
        && !text.contains("..")
        && !text.contains("//")
        && text
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'+' | b'/'))
}

/// 按来源装的插件信得过到什么程度（ADR-0017 的“信任和权限”）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Trust {
    /// 官方索引里审核过的版本，文件按索引记的 sha256 核对过。
    Official,
    /// GitHub 上的社区插件，没有审核过。
    #[default]
    Community,
}

/// 插件包是从哪里取来的，写进安装记录；上传的和预装的插件没有。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Origin {
    /// 不带版本的来源，如 `github:owner/repo` 或 `github:owner/repo/agents/codex`：商店和详情页用它。
    pub repo: String,
    /// 装的 tag；按提交装的为空。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
    pub commit: String,
    #[serde(default)]
    pub trust: Trust,
    /// 经第三方索引装的：它在索引里的来源，`index+<索引地址>#<插件 ID>`。升级看这个索引，不看仓库的 tag（ADR-0031）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub index: Option<String>,
}

impl Origin {
    /// 带着 tag 或提交的来源，如 `github:owner/repo@v1.2.0`：按它能再取到同一个插件包。
    pub fn source(&self) -> String {
        if let Some(index) = &self.index {
            return index.clone();
        }
        format!(
            "{}@{}",
            self.repo,
            self.tag.as_ref().unwrap_or(&self.commit)
        )
    }

    /// 经第三方索引装的：索引的主机。
    pub fn via_index(&self) -> Option<String> {
        // 只取主机名，语言无关；读不懂 index 来源时本来就不给。
        let (location, _) =
            IndexLocation::of_item_source(self.index.as_deref()?, &Languages::default()).ok()?;
        Some(location.provider())
    }

    /// 仓库里这个插件包所在位置的网页：目录链接，根目录时就是仓库。
    pub fn repository_url(&self) -> String {
        let base = format!("https://{}", self.provider());
        match self
            .repo
            .strip_prefix(SCHEME)
            .and_then(|p| p.splitn(3, '/').nth(2))
        {
            Some(subdir) => format!("{base}/tree/{}/{subdir}", self.commit),
            None => base,
        }
    }

    /// 核心能核实的提供方，如 `github.com/owner/repo`（ADR-0031）：子目录不影响。
    pub fn provider(&self) -> String {
        let path = self.repo.strip_prefix(SCHEME).unwrap_or(&self.repo);
        let mut parts = path.split('/');
        format!(
            "github.com/{}/{}",
            parts.next().unwrap_or_default(),
            parts.next().unwrap_or_default()
        )
    }
}

/// 把来源解析成提交：tag 经 `ls-refs` 查，提交号原样用。
pub async fn resolve(
    github: &GitHub,
    source: &SourceRef,
    langs: &Languages,
) -> Result<Origin, InstallError> {
    let repo = source.without_version();
    let (tag, commit) = match &source.wanted {
        Wanted::Commit(commit) => (None, commit.clone()),
        Wanted::Tag(name) => {
            let tags = github
                .tags(&source.repo, langs)
                .await
                .map_err(github_error)?;
            let tag = tags.into_iter().find(|t| &t.name == name).ok_or_else(|| {
                InstallError::NotFound(l10n::text(
                    langs,
                    "plugin-settings-github-no-tag",
                    &[
                        ("repo", source.repo.to_string().into()),
                        ("tag", name.as_str().into()),
                    ],
                ))
            })?;
            (Some(tag.name), tag.commit)
        }
        Wanted::Latest => {
            let tags = github
                .tags(&source.repo, langs)
                .await
                .map_err(github_error)?;
            let tag = latest(tags, source.tag_prefix()).ok_or_else(|| {
                InstallError::NotFound(l10n::text(
                    langs,
                    "plugin-settings-github-no-release",
                    &[("repo", source.repo.to_string().into())],
                ))
            })?;
            (Some(tag.name), tag.commit)
        }
    };
    Ok(Origin {
        repo,
        tag,
        commit,
        trust: Trust::Community,
        index: None,
    })
}

/// 版本号最大的正式版本 tag。带子目录时只看 `<前缀>/v<语义化版本>`。
pub fn latest(tags: Vec<Tag>, prefix: Option<&str>) -> Option<Tag> {
    tags.into_iter()
        .filter_map(|tag| Some((release_version(&tag.name, prefix)?, tag)))
        .max_by(|(a, _), (b, _)| a.cmp(b))
        .map(|(_, tag)| tag)
}

/// tag 里的版本号（不含 `v` 和子目录前缀）。带子目录时 tag 必须写成 `<前缀>/v<版本>`。
pub fn version_of<'a>(tag: &'a str, prefix: Option<&str>) -> Option<&'a str> {
    match prefix {
        Some(prefix) => tag.strip_prefix(prefix)?.strip_prefix("/v"),
        None => tag.strip_prefix('v'),
    }
}

/// `v<语义化版本>`、不带预发布后缀的 tag 的版本号。
fn release_version(tag: &str, prefix: Option<&str>) -> Option<semver::Version> {
    let version = semver::Version::parse(version_of(tag, prefix)?).ok()?;
    version.pre.is_empty().then_some(version)
}

/// 按提交取插件包：先取 compose，再取它引用的文件。按 tag 取的，清单的版本要和 tag 一致。
pub async fn fetch(
    github: &GitHub,
    origin: &Origin,
    langs: &Languages,
) -> Result<PluginPackage, InstallError> {
    let source = source_ref(origin)?;
    let repo = &source.repo;
    let prefix = source
        .subdir
        .as_ref()
        .map(|subdir| format!("{subdir}/"))
        .unwrap_or_default();
    let get = |path: String| {
        let full = format!("{prefix}{path}");
        async move {
            github
                .file(repo, &origin.commit, &full, MAX_FILE_BYTES, langs)
                .await
                .map_err(github_error)
        }
    };
    let compose = get(COMPOSE_FILE.to_owned()).await?;
    let compose = String::from_utf8(compose).map_err(|_| {
        InstallError::Invalid(crate::l10n::text(
            langs,
            "plugin-settings-upgrade-compose-not-utf8",
            &[("file", COMPOSE_FILE.into())],
        ))
    })?;
    let mut files = PackageFiles::new();
    let optional = PluginPackage::optional_files(&compose, langs)?;
    for path in PluginPackage::referenced_files(&compose, langs)? {
        let data = match get(path.clone()).await {
            Err(InstallError::NotFound(_)) if optional.contains(&path) => continue,
            result => result?,
        };
        files.insert(path, data);
    }
    let mut package = PluginPackage::with_files(&compose, files, langs)?;
    if let Some(root) = package.static_root() {
        let mut found = super::package::static_files::StaticFiles::new();
        let dir = format!("{prefix}{root}");
        for rel in github
            .tree(repo, &origin.commit, &dir, langs)
            .await
            .map_err(github_error)?
        {
            let path = format!("{dir}/{rel}");
            let data = github
                .file(
                    repo,
                    &origin.commit,
                    &path,
                    super::package::static_files::MAX_FILE_BYTES,
                    langs,
                )
                .await
                .map_err(github_error)?;
            found.insert(rel, data);
        }
        package = package.with_static(found, langs)?;
    }
    if let Some(tag) = &origin.tag {
        let version = &package.manifest.version;
        if version_of(tag, source.tag_prefix()) != Some(version.as_str()) {
            return Err(InstallError::Invalid(crate::l10n::text(
                langs,
                "plugin-settings-github-tag-version-mismatch",
                &[
                    ("tag", tag.as_str().into()),
                    ("version", version.as_str().into()),
                ],
            )));
        }
    }
    package.origin = Some(origin.clone());
    Ok(package)
}

fn source_ref(origin: &Origin) -> Result<SourceRef, InstallError> {
    SourceRef::parse(&origin.repo, &Languages::default())
}

pub(in crate::plugin) fn github_error(e: GitHubError) -> InstallError {
    match e {
        GitHubError::Unavailable(m) | GitHubError::Timeout(m) | GitHubError::RateLimited(m) => {
            InstallError::SourceUnavailable(m)
        }
        GitHubError::NotFound(m) => InstallError::NotFound(m),
        GitHubError::TooBig(m) => InstallError::Invalid(m),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugin::languages::Languages;

    fn tag(name: &str) -> Tag {
        Tag {
            name: name.to_owned(),
            commit: "0".repeat(40),
        }
    }

    #[test]
    fn sources_name_a_repo_and_optionally_a_tag_or_commit() {
        let parsed = SourceRef::parse("github:owner/repo.x@v1.2.0", &Languages::default()).unwrap();
        assert_eq!(parsed.repo.to_string(), "owner/repo.x");
        assert_eq!(parsed.wanted, Wanted::Tag("v1.2.0".into()));
        let commit = "a".repeat(40);
        assert_eq!(
            SourceRef::parse(&format!("github:o/r@{commit}"), &Languages::default())
                .unwrap()
                .wanted,
            Wanted::Commit(commit)
        );
        assert_eq!(
            SourceRef::parse("github:o/r", &Languages::default())
                .unwrap()
                .wanted,
            Wanted::Latest
        );
        for bad in [
            "https://github.com/o/r",
            "github:o",
            "github:../r",
            "github:o/r@",
            "github:o/r@../x",
            "github:o/r@a b",
        ] {
            assert!(
                SourceRef::parse(bad, &Languages::default()).is_err(),
                "{bad}"
            );
        }
    }

    /// 子目录可以有多段，加 tag 或提交号也能读回同样的来源；不合格的子目录按原因拒绝（完成标准 1）。
    #[test]
    fn a_subdirectory_round_trips_and_bad_ones_say_why() {
        let langs = Languages::default();
        let parsed = SourceRef::parse("github:o/r/agents/kimi-code", &langs).unwrap();
        assert_eq!(parsed.subdir.as_deref(), Some("agents/kimi-code"));
        assert_eq!(parsed.text(), "github:o/r/agents/kimi-code");
        let tagged =
            SourceRef::parse("github:o/r/agents/kimi-code@kimi-code/v0.1.0", &langs).unwrap();
        assert_eq!(tagged.wanted, Wanted::Tag("kimi-code/v0.1.0".into()));
        assert_eq!(
            tagged.text(),
            "github:o/r/agents/kimi-code@kimi-code/v0.1.0"
        );
        let commit =
            SourceRef::parse(&format!("github:o/r/sub@{}", "a".repeat(40)), &langs).unwrap();
        assert_eq!(commit.text(), format!("github:o/r/sub@{}", "a".repeat(40)));
        assert_eq!(tagged.tag_prefix(), Some("kimi-code"));
        assert_eq!(parsed.tag_prefix(), Some("kimi-code"));
        for bad in [
            "github:o/r//x",
            "github:o/r/x/",
            "github:o/r//",
            "github:o/r/",
            "github:o/r/./x",
            "github:o/r/x/../y",
            "github:o/r/x/a b",
            "github:o/r/x/y!",
        ] {
            let error = SourceRef::parse(bad, &langs).unwrap_err();
            assert_eq!(error.kind(), "invalid_input", "{bad}");
        }
        let long = "a".repeat(201);
        assert!(SourceRef::parse(&format!("github:o/r/{long}"), &langs).is_err());
        assert!(SourceRef::parse(&format!("github:o/r/{}", "a".repeat(200)), &langs).is_ok());
    }

    /// 同一仓库最后一段不能重复是规范的事，这里只证明同仓库的两个子目录是两个不同的来源。
    #[test]
    fn subdirectories_distinguish_sources_of_one_repository() {
        let langs = Languages::default();
        let one = SourceRef::parse("github:o/r/agents/codex", &langs).unwrap();
        let other = SourceRef::parse("github:o/r/apps/codex", &langs).unwrap();
        assert_ne!(one, other);
        assert_eq!(one.text(), "github:o/r/agents/codex");
    }

    #[test]
    fn the_latest_release_is_the_highest_version_without_a_prerelease() {
        let tags = [
            "v1.2.0",
            "v1.10.0",
            "v2.0.0-beta.1",
            "1.11.0",
            "latest",
            "v1.9.9",
        ];
        let found = latest(tags.into_iter().map(tag).collect(), None).unwrap();
        assert_eq!(found.name, "v1.10.0");
        assert!(latest(vec![tag("v1.0.0-rc.1"), tag("nightly")], None).is_none());
    }

    /// 带子目录时只看 `<子目录最后一段>/v<语义化版本>`（完成标准 4）。
    #[test]
    fn a_subdirectory_selects_its_own_prefixed_tags() {
        let tags = [
            "v9.0.0",
            "codex/v0.2.0",
            "codex/v0.3.0-beta.1",
            "kimi-code/v1.0.0",
        ]
        .into_iter()
        .map(tag)
        .collect();
        let found = latest(tags, Some("codex")).unwrap();
        assert_eq!(found.name, "codex/v0.2.0");
        assert_eq!(version_of("codex/v0.2.0", Some("codex")), Some("0.2.0"));
        assert_eq!(version_of("v9.0.0", Some("codex")), None);
        assert_eq!(version_of("v9.0.0", None), Some("9.0.0"));
    }
}
