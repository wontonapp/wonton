//! 按来源取插件包（插件规范的“来源”）：`github:<owner>/<repo>`，可以带 `@<tag>` 或 `@<40 位提交号>`。先把来源解析成
//! 提交，再按提交取 `docker-compose.yml` 和它引用的文件，和上传的插件包走同一套解析和校验。

use serde::{Deserialize, Serialize};

use super::github::{GitHub, GitHubError, Repo, Tag, is_commit_id};
use super::index_location::IndexLocation;
use super::install_error::InstallError;
use super::package::PluginPackage;
use super::package_files::{COMPOSE_FILE, MAX_FILE_BYTES, PackageFiles};

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
    pub wanted: Wanted,
}

impl SourceRef {
    pub fn parse(text: &str) -> Result<Self, InstallError> {
        let invalid = |why: &str| {
            InstallError::Invalid(format!(
                "来源 `{text}` 不对：{why}；写成 github:<owner>/<repo>，可以带 @<tag> 或 @<40 位提交号>"
            ))
        };
        let rest = text
            .strip_prefix(SCHEME)
            .ok_or_else(|| invalid("只支持 github: 开头的来源"))?;
        let (path, version) = match rest.split_once('@') {
            Some((path, version)) => (path, Some(version)),
            None => (rest, None),
        };
        let (owner, name) = path.split_once('/').ok_or_else(|| invalid("缺少仓库名"))?;
        if !is_name(owner) || !is_name(name) {
            return Err(invalid("owner 和仓库名只能有字母、数字、`-`、`_`、`.`"));
        }
        let wanted = match version {
            None => Wanted::Latest,
            Some(v) if is_commit_id(v) => Wanted::Commit(v.to_owned()),
            Some(v) if is_tag(v) => Wanted::Tag(v.to_owned()),
            Some(_) => return Err(invalid("@ 后面的 tag 不合法")),
        };
        let repo = Repo {
            owner: owner.to_owned(),
            name: name.to_owned(),
        };
        Ok(Self { repo, wanted })
    }
}

impl SourceRef {
    /// 来源字符串，[`Self::parse`] 能读回来。
    pub fn text(&self) -> String {
        match &self.wanted {
            Wanted::Latest => format!("{SCHEME}{}", self.repo),
            Wanted::Tag(version) | Wanted::Commit(version) => {
                format!("{SCHEME}{}@{version}", self.repo)
            }
        }
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

/// tag 名：git 允许的字符很多，这里只收发布用得到的。
fn is_tag(text: &str) -> bool {
    !text.is_empty()
        && text.len() <= 128
        && !text.starts_with(['.', '-'])
        && !text.contains("..")
        && text
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'+'))
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
    /// 不带版本的来源，如 `github:owner/repo`：商店和详情页用它。
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
        let (location, _) = IndexLocation::of_item_source(self.index.as_deref()?).ok()?;
        Some(location.provider())
    }

    /// 仓库的网页。
    pub fn repository_url(&self) -> String {
        format!("https://{}", self.provider())
    }

    /// 核心能核实的提供方，如 `github.com/owner/repo`（ADR-0031）。
    pub fn provider(&self) -> String {
        let path = self.repo.strip_prefix(SCHEME).unwrap_or(&self.repo);
        format!("github.com/{path}")
    }
}

/// 把来源解析成提交：tag 经 `ls-refs` 查，提交号原样用。
pub async fn resolve(github: &GitHub, source: &SourceRef) -> Result<Origin, InstallError> {
    let repo = format!("{SCHEME}{}", source.repo);
    let (tag, commit) = match &source.wanted {
        Wanted::Commit(commit) => (None, commit.clone()),
        Wanted::Tag(name) => {
            let tags = github.tags(&source.repo).await.map_err(github_error)?;
            let tag = tags.into_iter().find(|t| &t.name == name).ok_or_else(|| {
                InstallError::NotFound(format!("{} 没有 tag {name}", source.repo))
            })?;
            (Some(tag.name), tag.commit)
        }
        Wanted::Latest => {
            let tags = github.tags(&source.repo).await.map_err(github_error)?;
            let tag = latest(tags).ok_or_else(|| {
                InstallError::NotFound(format!(
                    "{} 没有正式版本：要有 v<语义化版本> 的 tag，如 v1.0.0",
                    source.repo
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

/// 版本号最大的正式版本 tag。
pub fn latest(tags: Vec<Tag>) -> Option<Tag> {
    tags.into_iter()
        .filter_map(|tag| Some((release_version(&tag.name)?, tag)))
        .max_by(|(a, _), (b, _)| a.cmp(b))
        .map(|(_, tag)| tag)
}

/// `v<语义化版本>`、不带预发布后缀的 tag 的版本号。
fn release_version(tag: &str) -> Option<semver::Version> {
    let version = semver::Version::parse(tag.strip_prefix('v')?).ok()?;
    version.pre.is_empty().then_some(version)
}

/// 按提交取插件包：先取 compose，再取它引用的文件。按 tag 取的，清单的版本要和 tag 一致。
pub async fn fetch(github: &GitHub, origin: &Origin) -> Result<PluginPackage, InstallError> {
    let repo = source_repo(origin)?;
    let get = |path: String| {
        let repo = &repo;
        async move {
            github
                .file(repo, &origin.commit, &path, MAX_FILE_BYTES)
                .await
                .map_err(github_error)
        }
    };
    let compose = get(COMPOSE_FILE.to_owned()).await?;
    let compose = String::from_utf8(compose)
        .map_err(|_| InstallError::Invalid(format!("{COMPOSE_FILE} 不是 UTF-8 文本")))?;
    let mut files = PackageFiles::new();
    for path in PluginPackage::referenced_files(&compose)? {
        let data = get(path.clone()).await?;
        files.insert(path, data);
    }
    let mut package = PluginPackage::with_files(&compose, files)?;
    if let Some(tag) = &origin.tag {
        let version = &package.manifest.version;
        if tag.strip_prefix('v') != Some(version.as_str()) {
            return Err(InstallError::Invalid(format!(
                "tag {tag} 和清单的版本 {version} 不一致：发布时 tag 要是 v<清单的版本>"
            )));
        }
    }
    package.origin = Some(origin.clone());
    Ok(package)
}

fn source_repo(origin: &Origin) -> Result<Repo, InstallError> {
    Ok(SourceRef::parse(&origin.repo)?.repo)
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

    fn tag(name: &str) -> Tag {
        Tag {
            name: name.to_owned(),
            commit: "0".repeat(40),
        }
    }

    #[test]
    fn sources_name_a_repo_and_optionally_a_tag_or_commit() {
        let parsed = SourceRef::parse("github:owner/repo.x@v1.2.0").unwrap();
        assert_eq!(parsed.repo.to_string(), "owner/repo.x");
        assert_eq!(parsed.wanted, Wanted::Tag("v1.2.0".into()));
        let commit = "a".repeat(40);
        assert_eq!(
            SourceRef::parse(&format!("github:o/r@{commit}"))
                .unwrap()
                .wanted,
            Wanted::Commit(commit)
        );
        assert_eq!(
            SourceRef::parse("github:o/r").unwrap().wanted,
            Wanted::Latest
        );
        for bad in [
            "https://github.com/o/r",
            "github:o",
            "github:o/r/x",
            "github:../r",
            "github:o/r@",
            "github:o/r@../x",
            "github:o/r@a b",
        ] {
            assert!(SourceRef::parse(bad).is_err(), "{bad}");
        }
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
        let found = latest(tags.into_iter().map(tag).collect()).unwrap();
        assert_eq!(found.name, "v1.10.0");
        assert!(latest(vec![tag("v1.0.0-rc.1"), tag("nightly")]).is_none());
    }
}
