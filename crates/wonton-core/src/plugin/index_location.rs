//! 第三方索引在哪里，和索引里的插件的来源 `index+<索引地址>#<插件 ID>`（ADR-0031）。

use url::Url;

use super::github::{GitHub, GitHubError, Repo};
use super::github_source::{INDEX_SCHEME, SourceRef, Wanted};
use super::install_error::InstallError;

const RAW_HOST: &str = "raw.githubusercontent.com";
pub const INDEX_FILE: &str = "index.json";

/// 第三方索引的位置。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IndexLocation {
    /// 仓库根目录的 `index.json`，跟着默认分支。
    Repo(Repo),
    Url(Url),
}

impl IndexLocation {
    /// 规范后的地址。只认 https：未审核的来源要靠 TLS 确认内容没被改过。
    pub fn parse(text: &str) -> Result<Self, InstallError> {
        let invalid = |why: String| InstallError::Invalid(format!("索引地址 `{text}` 不对：{why}"));
        let url = Url::parse(text).map_err(|e| invalid(e.to_string()))?;
        if url.scheme() != "https" {
            return Err(invalid("只支持 https".to_owned()));
        }
        if url.host_str() == Some(RAW_HOST) {
            let parts: Vec<&str> = url.path().split('/').filter(|p| !p.is_empty()).collect();
            if let [owner, name, "HEAD", INDEX_FILE] = parts.as_slice() {
                let source = SourceRef::parse(&format!("github:{owner}/{name}"))?;
                debug_assert_eq!(source.wanted, Wanted::Latest);
                return Ok(Self::Repo(source.repo));
            }
        }
        Ok(Self::Url(url))
    }

    pub fn address(&self) -> String {
        match self {
            Self::Repo(repo) => format!("https://{RAW_HOST}/{repo}/HEAD/{INDEX_FILE}"),
            Self::Url(url) => url.to_string(),
        }
    }

    /// 给主人看的提供方：`github.com/owner/repo`，或者索引所在的主机。
    pub fn provider(&self) -> String {
        match self {
            Self::Repo(repo) => format!("github.com/{repo}"),
            Self::Url(url) => url.host_str().unwrap_or_default().to_owned(),
        }
    }

    /// 索引里一个插件的来源。
    pub fn item_source(&self, id: &str) -> String {
        format!("{INDEX_SCHEME}{}#{id}", self.address())
    }

    /// `index+<索引地址>#<插件 ID>` → 索引的位置和插件 ID。
    pub fn of_item_source(source: &str) -> Result<(Self, &str), InstallError> {
        let invalid = || InstallError::Invalid(format!("来源 `{source}` 不对"));
        let rest = source.strip_prefix(INDEX_SCHEME).ok_or_else(invalid)?;
        let (address, id) = rest.rsplit_once('#').ok_or_else(invalid)?;
        Ok((Self::parse(address)?, id))
    }

    /// 索引文件。
    pub async fn read_index(&self, github: &GitHub, max: u64) -> Result<Vec<u8>, GitHubError> {
        self.read(github, INDEX_FILE, max).await
    }

    /// 索引旁边的文件，`path` 相对索引所在的目录。
    pub async fn read(
        &self,
        github: &GitHub,
        path: &str,
        max: u64,
    ) -> Result<Vec<u8>, GitHubError> {
        match self {
            Self::Repo(repo) => github.file(repo, "HEAD", path, max).await,
            Self::Url(url) if path == INDEX_FILE => github.url(url, max).await,
            Self::Url(url) => {
                let sibling = url.join(path).map_err(|e| {
                    GitHubError::NotFound(format!("{url} 旁边的 {path} 不是合法地址：{e}"))
                })?;
                github.url(&sibling, max).await
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_index_in_a_repository_follows_the_default_branch_and_names_the_repository() {
        let text = "https://raw.githubusercontent.com/o/r/HEAD/index.json";
        let location = IndexLocation::parse(text).unwrap();
        assert!(matches!(&location, IndexLocation::Repo(repo) if repo.to_string() == "o/r"));
        assert_eq!(location.address(), text);
        assert_eq!(location.provider(), "github.com/o/r");
    }

    #[test]
    fn an_item_source_finds_its_index_and_plugin_again() {
        let location = IndexLocation::parse("https://example.com/wonton/index.json").unwrap();
        assert_eq!(location.provider(), "example.com");
        let source = location.item_source("app.hello");
        assert_eq!(
            source,
            "index+https://example.com/wonton/index.json#app.hello"
        );
        let (found, id) = IndexLocation::of_item_source(&source).unwrap();
        assert_eq!(found, location);
        assert_eq!(id, "app.hello");
    }

    #[test]
    fn only_https_is_accepted() {
        assert!(IndexLocation::parse("http://example.com/index.json").is_err());
        assert!(IndexLocation::of_item_source("index+http://example.com/i.json#a.b").is_err());
    }
}
