//! 主人在“添加来源”里填的地址（ADR-0031）：认出是 GitHub 仓库还是 https 地址。仓库是插件还是索引，要看仓库的内容。

use url::Url;

use super::github_source::{INDEX_SCHEME, SourceRef};
use super::install_error::InstallError;
use super::languages::Languages;
use super::staged;

const GITHUB_HOSTS: [&str; 2] = ["github.com", "www.github.com"];

/// 地址说的是什么。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Address {
    Repo(SourceRef),
    /// GitHub 之外的 https 地址，只能是索引。
    Url(Url),
}

impl Address {
    pub fn parse(text: &str, langs: &Languages) -> Result<Self, InstallError> {
        let text = text.trim();
        let invalid = |why: String| {
            InstallError::Invalid(crate::l10n::text(
                langs,
                "plugin-settings-source-address-invalid",
                &[("text", text.into()), ("why", why.as_str().into())],
            ))
        };
        if text.starts_with("github:") {
            return SourceRef::parse(text, langs).map(Self::Repo);
        }
        if let Some(rest) = text.strip_prefix("http://") {
            let path = GITHUB_HOSTS.iter().find_map(|h| rest.strip_prefix(h));
            return match path {
                Some(path) => github_page(path, langs).map(Self::Repo),
                None => Err(invalid(crate::l10n::text(
                    langs,
                    "plugin-settings-source-address-tls",
                    &[],
                ))),
            };
        }
        if text.starts_with("https://") {
            let url = Url::parse(text).map_err(|e| invalid(e.to_string()))?;
            let host = url.host_str().unwrap_or_default();
            if GITHUB_HOSTS.contains(&host) {
                return github_page(url.path(), langs).map(Self::Repo);
            }
            return Ok(Self::Url(url));
        }
        if let Some(path) = GITHUB_HOSTS.iter().find_map(|h| text.strip_prefix(h)) {
            return github_page(path, langs).map(Self::Repo);
        }
        if text.contains("://") {
            return Err(invalid(crate::l10n::text(
                langs,
                "plugin-settings-source-address-https-only",
                &[],
            )));
        }
        SourceRef::parse(&format!("github:{text}"), langs)
            .map(Self::Repo)
            .map_err(|_| {
                invalid(crate::l10n::text(
                    langs,
                    "plugin-settings-source-address-formats",
                    &[],
                ))
            })
    }
}

/// 换版本时主人填的来源：各种仓库写法都认，规范成来源字符串。App 从预览和商店拿到的来源原样过去。
pub fn chosen_source(text: &str, langs: &Languages) -> Result<String, InstallError> {
    let text = text.trim();
    if text.starts_with(INDEX_SCHEME) || text.starts_with(staged::SCHEME) {
        return Ok(text.to_owned());
    }
    match Address::parse(text, langs)? {
        Address::Repo(source) => Ok(source.text()),
        Address::Url(_) => Err(InstallError::Invalid(crate::l10n::text(
            langs,
            "plugin-settings-source-not-a-repo",
            &[("text", text.into())],
        ))),
    }
}

/// GitHub 上的页面路径 → 来源：`/o/r`、`/o/r.git`、`/o/r/tree/<tag>[/<子目录>]`、`/o/r/releases/tag/<tag>`、
/// `/o/r/commit/<提交>`。
fn github_page(path: &str, langs: &Languages) -> Result<SourceRef, InstallError> {
    let path = path.split(['?', '#']).next().unwrap_or_default();
    let parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
    let (owner, name, subdir, version) = match parts.as_slice() {
        [owner, name] => (*owner, *name, None, None),
        [owner, name, "tree" | "commit", version] => (*owner, *name, None, Some(*version)),
        [owner, name, "releases", "tag", version] => (*owner, *name, None, Some(*version)),
        [owner, name, "tree", version, subdir @ ..] => {
            (*owner, *name, Some(subdir.join("/")), Some(*version))
        }
        _ => {
            return Err(InstallError::Invalid(crate::l10n::text(
                langs,
                "plugin-settings-source-page-invalid",
                &[("path", path.into())],
            )));
        }
    };
    let name = name.strip_suffix(".git").unwrap_or(name);
    let subdir = subdir.map(|s| format!("/{s}")).unwrap_or_default();
    let version = version.map(|v| format!("@{v}")).unwrap_or_default();
    SourceRef::parse(&format!("github:{owner}/{name}{subdir}{version}"), langs)
}

#[cfg(test)]
mod tests {
    use super::super::github_source::Wanted;
    use super::*;
    use crate::plugin::languages::Languages;

    fn repo(text: &str) -> (String, Wanted) {
        match Address::parse(text, &Languages::default()).unwrap() {
            Address::Repo(source) => (source.repo.to_string(), source.wanted),
            Address::Url(url) => panic!("{text} 认成了地址 {url}"),
        }
    }

    #[test]
    fn every_way_of_writing_a_repository_means_the_same_repository() {
        for text in [
            "github:o/r",
            "o/r",
            " o/r\n",
            "github.com/o/r",
            "https://github.com/o/r",
            "https://github.com/o/r/",
            "https://github.com/o/r.git",
            "https://www.github.com/o/r#readme",
            "http://github.com/o/r",
        ] {
            assert_eq!(repo(text), ("o/r".to_owned(), Wanted::Latest), "{text}");
        }
    }

    #[test]
    fn a_page_of_a_tag_or_commit_names_that_version() {
        let tag = ("o/r".to_owned(), Wanted::Tag("v1.2.0".to_owned()));
        for text in [
            "github:o/r@v1.2.0",
            "o/r@v1.2.0",
            "https://github.com/o/r/tree/v1.2.0",
            "https://github.com/o/r/releases/tag/v1.2.0",
        ] {
            assert_eq!(repo(text), tag, "{text}");
        }
        let commit = "a".repeat(40);
        assert_eq!(
            repo(&format!("https://github.com/o/r/commit/{commit}")),
            ("o/r".to_owned(), Wanted::Commit(commit))
        );
    }

    /// `/tree/<ref>/<路径>` 解析成带子目录的来源，`<ref>` 是 tag 或提交（完成标准 2）。
    #[test]
    fn a_tree_page_with_a_subdirectory_names_that_source() {
        let langs = Languages::default();
        let parsed =
            match Address::parse("https://github.com/o/r/tree/v1.0.0/agents/x", &langs).unwrap() {
                Address::Repo(source) => source,
                other => panic!("{other:?}"),
            };
        assert_eq!(
            parsed,
            SourceRef::parse("github:o/r/agents/x@v1.0.0", &langs).unwrap()
        );
        // 多段子目录也一样。
        let deep = match Address::parse("https://github.com/o/r/tree/main/a/b/c", &langs).unwrap() {
            Address::Repo(source) => source,
            other => panic!("{other:?}"),
        };
        assert_eq!(deep.subdir.as_deref(), Some("a/b/c"));
        assert_eq!(deep.wanted, Wanted::Tag("main".to_owned()));
    }

    #[test]
    fn other_https_addresses_are_indexes_and_the_rest_is_refused() {
        let url = "https://example.com/wonton/index.json";
        assert_eq!(
            Address::parse(url, &Languages::default()).unwrap(),
            Address::Url(Url::parse(url).unwrap())
        );
        for bad in [
            "http://example.com/index.json",
            "ftp://example.com/index.json",
            "file:///etc/passwd",
            "https://github.com/o",
            "https://github.com/o/r/blob/main/README.md",
            "not an address",
            "",
        ] {
            assert!(Address::parse(bad, &Languages::default()).is_err(), "{bad}");
        }
    }
}
