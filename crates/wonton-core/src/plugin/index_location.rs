//! 第三方索引在哪里，和索引里的插件的来源 `index+<索引地址>#<插件 ID>`（ADR-0031）。

use url::Url;

use super::github::{GitHub, GitHubError, Repo};
use super::github_source::{INDEX_SCHEME, SourceRef, Wanted};
use super::install_error::InstallError;
use super::languages::Languages;
use crate::l10n;

const RAW_HOST: &str = "raw.githubusercontent.com";
pub const INDEX_FILE: &str = "index.json";

/// 第三方索引的位置。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IndexLocation {
    /// 仓库里的 `index.json`（可以在子目录里），跟着默认分支。
    Repo {
        repo: Repo,
        subdir: Option<String>,
    },
    Url(Url),
}

impl IndexLocation {
    /// 规范后的地址。只认 https：未审核的来源要靠 TLS 确认内容没被改过。
    pub fn parse(text: &str, langs: &Languages) -> Result<Self, InstallError> {
        let invalid = |why: String| {
            InstallError::Invalid(crate::l10n::text(
                langs,
                "plugin-settings-index-address-invalid",
                &[("text", text.into()), ("why", why.into())],
            ))
        };
        let url = Url::parse(text).map_err(|e| invalid(e.to_string()))?;
        if url.scheme() != "https" {
            return Err(invalid(crate::l10n::text(
                langs,
                "plugin-settings-index-address-https",
                &[],
            )));
        }
        if url.host_str() == Some(RAW_HOST) {
            let parts: Vec<&str> = url.path().split('/').filter(|p| !p.is_empty()).collect();
            if let [owner, name, "HEAD", rest @ ..] = parts.as_slice()
                && rest.last() == Some(&INDEX_FILE)
            {
                let dir = &rest[..rest.len() - 1];
                let source = match dir {
                    [] => SourceRef::parse(&format!("github:{owner}/{name}"), langs)?,
                    dir => SourceRef::parse(
                        &format!("github:{owner}/{name}/{}", dir.join("/")),
                        langs,
                    )?,
                };
                debug_assert_eq!(source.wanted, Wanted::Latest);
                return Ok(Self::Repo {
                    repo: source.repo,
                    subdir: source.subdir,
                });
            }
        }
        Ok(Self::Url(url))
    }

    pub fn address(&self) -> String {
        match self {
            Self::Repo { repo, subdir } => {
                let dir = subdir.as_ref().map_or(String::new(), |s| format!("/{s}"));
                format!("https://{RAW_HOST}/{repo}/HEAD{dir}/{INDEX_FILE}")
            }
            Self::Url(url) => url.to_string(),
        }
    }

    /// 给主人看的提供方：`github.com/owner/repo`，或者索引所在的主机。
    pub fn provider(&self) -> String {
        match self {
            Self::Repo { repo, .. } => format!("github.com/{repo}"),
            Self::Url(url) => url.host_str().unwrap_or_default().to_owned(),
        }
    }

    /// 索引里一个插件的来源。
    pub fn item_source(&self, id: &str) -> String {
        format!("{INDEX_SCHEME}{}#{id}", self.address())
    }

    /// `index+<索引地址>#<插件 ID>` → 索引的位置和插件 ID。
    pub fn of_item_source<'a>(
        source: &'a str,
        langs: &Languages,
    ) -> Result<(Self, &'a str), InstallError> {
        let invalid = || {
            InstallError::Invalid(crate::l10n::text(
                langs,
                "plugin-settings-item-source-invalid",
                &[("source", source.into())],
            ))
        };
        let rest = source.strip_prefix(INDEX_SCHEME).ok_or_else(invalid)?;
        let (address, id) = rest.rsplit_once('#').ok_or_else(invalid)?;
        Ok((Self::parse(address, langs)?, id))
    }

    /// 索引文件。
    pub async fn read_index(
        &self,
        github: &GitHub,
        max: u64,
        langs: &Languages,
    ) -> Result<Vec<u8>, GitHubError> {
        self.read(github, INDEX_FILE, max, langs).await
    }

    /// 索引旁边的文件，`path` 相对索引所在的目录。
    pub async fn read(
        &self,
        github: &GitHub,
        path: &str,
        max: u64,
        langs: &Languages,
    ) -> Result<Vec<u8>, GitHubError> {
        match self {
            Self::Repo { repo, subdir } => {
                let full = subdir
                    .as_ref()
                    .map_or_else(|| path.to_owned(), |subdir| format!("{subdir}/{path}"));
                github.file(repo, "HEAD", &full, max, langs).await
            }
            Self::Url(url) if path == INDEX_FILE => github.url(url, max, langs).await,
            Self::Url(url) => {
                let sibling = url.join(path).map_err(|e| {
                    GitHubError::NotFound(l10n::text(
                        langs,
                        "plugin-settings-github-sibling-url",
                        &[
                            ("url", url.as_str().into()),
                            ("path", path.into()),
                            ("error", e.to_string().into()),
                        ],
                    ))
                })?;
                github.url(&sibling, max, langs).await
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugin::languages::Languages;

    #[test]
    fn an_index_in_a_repository_follows_the_default_branch_and_names_the_repository() {
        let text = "https://raw.githubusercontent.com/o/r/HEAD/index.json";
        let location = IndexLocation::parse(text, &Languages::default()).unwrap();
        assert!(matches!(
            &location,
            IndexLocation::Repo { repo, subdir: None } if repo.to_string() == "o/r"
        ));
        assert_eq!(location.address(), text);
        assert_eq!(location.provider(), "github.com/o/r");
    }

    #[test]
    fn an_item_source_finds_its_index_and_plugin_again() {
        let location = IndexLocation::parse(
            "https://example.com/wonton/index.json",
            &Languages::default(),
        )
        .unwrap();
        assert_eq!(location.provider(), "example.com");
        let source = location.item_source("app.hello");
        assert_eq!(
            source,
            "index+https://example.com/wonton/index.json#app.hello"
        );
        let (found, id) = IndexLocation::of_item_source(&source, &Languages::default()).unwrap();
        assert_eq!(found, location);
        assert_eq!(id, "app.hello");
    }

    #[test]
    fn only_https_is_accepted() {
        assert!(
            IndexLocation::parse("http://example.com/index.json", &Languages::default()).is_err()
        );
        assert!(
            IndexLocation::of_item_source(
                "index+http://example.com/i.json#a.b",
                &Languages::default()
            )
            .is_err()
        );
    }
}
