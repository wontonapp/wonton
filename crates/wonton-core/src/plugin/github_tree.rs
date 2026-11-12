//! 某个提交里一个目录下的全部文件路径：静态网页的目录不能从 compose 推出来（ADR-0080），要列一次目录树。
//! 只有 API 能列（raw 不能），不带令牌时每个出口 IP 每小时 60 次：每次取包只列一次。

use reqwest::StatusCode;

use super::{GitHub, GitHubError, Repo, missing, read_limited, unavailable};
use crate::l10n;
use crate::plugin::languages::Languages;

/// 目录树响应的上限：1024 项的树远小于它。
const MAX_TREE_BYTES: u64 = 8 << 20;

#[derive(serde::Deserialize)]
struct Tree {
    tree: Vec<TreeItem>,
    #[serde(default)]
    truncated: bool,
}

#[derive(serde::Deserialize)]
struct TreeItem {
    path: String,
    #[serde(rename = "type")]
    kind: String,
    mode: String,
}

impl GitHub {
    /// 提交 `commit` 里 `dir` 下的全部文件，路径相对 `dir`。软链接（mode 120000）和子模块报错，不跟进去。
    pub async fn tree(
        &self,
        repo: &Repo,
        commit: &str,
        dir: &str,
        langs: &Languages,
    ) -> Result<Vec<String>, GitHubError> {
        let what = l10n::text(
            langs,
            "plugin-settings-github-tree-what",
            &[
                ("repo", repo.to_string().into()),
                ("commit", commit.into()),
                ("dir", dir.into()),
            ],
        );
        let response = self
            .client
            .get(format!(
                "{}/repos/{repo}/git/trees/{commit}?recursive=1",
                self.api
            ))
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .send()
            .await
            .map_err(|e| unavailable(&what, &e, langs))?;
        match response.status() {
            StatusCode::OK => {}
            StatusCode::NOT_FOUND => return Err(GitHubError::NotFound(missing(langs, &what))),
            StatusCode::FORBIDDEN | StatusCode::TOO_MANY_REQUESTS => {
                return Err(GitHubError::RateLimited(l10n::text(
                    langs,
                    "plugin-settings-github-rate-limited",
                    &[("what", what.as_str().into())],
                )));
            }
            status => {
                return Err(GitHubError::Unavailable(l10n::text(
                    langs,
                    "plugin-settings-github-api-status",
                    &[
                        ("what", what.as_str().into()),
                        ("status", format!("{status}").into()),
                    ],
                )));
            }
        }
        let body = read_limited(response, MAX_TREE_BYTES, langs)
            .await
            .map_err(|e| e.context(&what, langs))?;
        let unreadable = |error: String| {
            GitHubError::Unavailable(l10n::text(
                langs,
                "plugin-settings-github-context",
                &[("what", what.as_str().into()), ("error", error.into())],
            ))
        };
        let tree: Tree = serde_json::from_slice(&body).map_err(|e| unreadable(e.to_string()))?;
        if tree.truncated {
            return Err(unreadable("GitHub truncated the tree".to_owned()));
        }
        files_under(tree.tree, dir).map_err(unreadable)
    }
}

fn files_under(items: Vec<TreeItem>, dir: &str) -> Result<Vec<String>, String> {
    let prefix = format!("{dir}/");
    let mut files = Vec::new();
    for item in items {
        let Some(rel) = item.path.strip_prefix(&prefix) else {
            continue;
        };
        match (item.kind.as_str(), item.mode.as_str()) {
            ("tree", _) => {}
            ("blob", "120000") => return Err(format!("`{}` is a symlink", item.path)),
            ("blob", _) => files.push(rel.to_owned()),
            (kind, _) => return Err(format!("`{}` is a {kind}, not a file", item.path)),
        }
    }
    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(path: &str, kind: &str, mode: &str) -> TreeItem {
        TreeItem {
            path: path.to_owned(),
            kind: kind.to_owned(),
            mode: mode.to_owned(),
        }
    }

    #[test]
    fn only_files_under_the_directory_are_listed_and_links_are_refused() {
        let items = vec![
            item("docker-compose.yml", "blob", "100644"),
            item("web", "tree", "040000"),
            item("web/index.html", "blob", "100644"),
            item("web/js", "tree", "040000"),
            item("web/js/app.js", "blob", "100755"),
            item("website/x", "blob", "100644"),
        ];
        assert_eq!(
            files_under(items, "web").unwrap(),
            vec!["index.html", "js/app.js"]
        );
        let link = vec![item("web/x", "blob", "120000")];
        assert!(files_under(link, "web").unwrap_err().contains("symlink"));
        let module = vec![item("web/sub", "commit", "160000")];
        assert!(files_under(module, "web").is_err());
    }
}
