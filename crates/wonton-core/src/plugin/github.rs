//! 从 GitHub 上的公开仓库取插件包（插件规范的“来源”）：tag 经 git 协议 v2 的 `ls-refs` 解析成提交，文件按提交从
//! `raw.githubusercontent.com` 取。两者都不计入 GitHub API 的额度（不带令牌时每小时 60 次），核心也不存 GitHub 令牌。
//! 下载经主人的代理（[`CoreProxy`]）。

use std::time::Duration;

use reqwest::StatusCode;

use super::egress::core_proxy::CoreProxy;

/// 一次请求最多等这么久：GitHub 在国内时常很慢，但不能让预览一直挂着。
const TIMEOUT: Duration = Duration::from_secs(30);
// hyper-util 按地址分摊连接时限；不设时，首个不通的 IPv4 会耗尽整个请求期限，后续地址没有机会。
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
/// `ls-refs` 的响应最多这么大：一个 tag 一行，几万个 tag 也用不到。
const MAX_REFS_BYTES: u64 = 4 << 20;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum GitHubError {
    /// 连不上 GitHub，或者 GitHub 出错了。
    #[error("{0}")]
    Unavailable(String),
    /// 没有在时限内答完。
    #[error("{0}")]
    Timeout(String),
    /// 没有这个公开仓库，或者这个提交里没有这个文件。
    #[error("{0}")]
    NotFound(String),
    /// 文件超过了给的上限。
    #[error("{0}")]
    TooBig(String),
    /// GitHub 限速了（搜索 API）。
    #[error("{0}")]
    RateLimited(String),
}

/// GitHub 上的一个仓库：`owner/name`。
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Repo {
    pub owner: String,
    pub name: String,
}

impl std::fmt::Display for Repo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}/{}", self.owner, self.name)
    }
}

/// 仓库里的一个 tag：名字（不带 `refs/tags/`）和它指向的提交（注解 tag 已经剥到提交）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tag {
    pub name: String,
    pub commit: String,
}

#[derive(Debug, Clone)]
pub struct GitHub {
    client: reqwest::Client,
    /// `https://github.com`，git 协议走这里。
    git: String,
    /// `https://raw.githubusercontent.com`。
    raw: String,
    /// `https://api.github.com`：搜索社区插件，raw 不通时取官方索引。
    api: String,
}

impl GitHub {
    pub fn new(proxy: &CoreProxy) -> Self {
        let client = client(proxy).build().expect("GitHub 客户端的配置是固定的");
        Self {
            client,
            git: "https://github.com".to_owned(),
            raw: "https://raw.githubusercontent.com".to_owned(),
            api: "https://api.github.com".to_owned(),
        }
    }

    /// 连到测试替身：两个地址都是它，不走代理。
    #[cfg(test)]
    pub fn for_test(base: &str) -> Self {
        let client = client(&CoreProxy::default())
            .timeout(Duration::from_secs(5))
            .no_proxy()
            .build()
            .unwrap();
        Self {
            client,
            git: base.to_owned(),
            raw: base.to_owned(),
            api: base.to_owned(),
        }
    }

    /// 仓库的全部 tag。仓库不存在或不公开时 GitHub 要求登录（401），这里报 [`GitHubError::NotFound`]。
    pub async fn tags(&self, repo: &Repo) -> Result<Vec<Tag>, GitHubError> {
        let url = format!("{}/{repo}.git/git-upload-pack", self.git);
        let response = self
            .client
            .post(&url)
            .header("Content-Type", "application/x-git-upload-pack-request")
            .header("Accept", "application/x-git-upload-pack-result")
            .header("Git-Protocol", "version=2")
            .body(ls_refs_request())
            .send()
            .await
            .map_err(|e| unavailable(&format!("读 {repo} 的 tag"), &e))?;
        match response.status() {
            StatusCode::OK => {}
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN | StatusCode::NOT_FOUND => {
                return Err(GitHubError::NotFound(format!(
                    "GitHub 上没有公开仓库 {repo}"
                )));
            }
            status => {
                return Err(GitHubError::Unavailable(format!(
                    "读 {repo} 的 tag 时 GitHub 回了 {status}"
                )));
            }
        }
        let body = read_limited(response, MAX_REFS_BYTES)
            .await
            .map_err(|e| e.context(&format!("读 {repo} 的 tag")))?;
        parse_ls_refs(&body)
            .map_err(|e| GitHubError::Unavailable(format!("GitHub 给的 {repo} 的 tag 读不懂：{e}")))
    }

    /// 某个提交里的一个文件，超过 `max` 字节时报错。
    pub async fn file(
        &self,
        repo: &Repo,
        commit: &str,
        path: &str,
        max: u64,
    ) -> Result<Vec<u8>, GitHubError> {
        let url = format!("{}/{repo}/{commit}/{path}", self.raw);
        let what = format!("取 {repo} 在提交 {commit} 里的 {path}");
        let result = self.file_once(&url, &what, max).await;
        // 直连会偶发卡在首字节或正文。只有内容不变的提交可以整文件重取；丢弃半个响应，仍核对大小和摘要。
        if is_commit_id(commit) && matches!(&result, Err(GitHubError::Timeout(_))) {
            tracing::info!(%repo, %commit, %path, "取文件超时，重取一次");
            return self.file_once(&url, &what, max).await;
        }
        result
    }

    async fn file_once(&self, url: &str, what: &str, max: u64) -> Result<Vec<u8>, GitHubError> {
        let response = self
            .client
            .get(url)
            .send()
            .await
            .map_err(|e| unavailable(what, &e))?;
        match response.status() {
            StatusCode::OK => read_limited(response, max)
                .await
                .map_err(|e| e.context(what)),
            StatusCode::NOT_FOUND => Err(GitHubError::NotFound(format!("没有 {what}"))),
            status => Err(GitHubError::Unavailable(format!(
                "{what} 时 GitHub 回了 {status}"
            ))),
        }
    }
}

impl GitHub {
    /// 任意地址的文件，超过 `max` 字节时报错：第三方索引可以放在任何静态托管上。走同一个客户端，代理和超时都一样。
    pub async fn url(&self, url: &reqwest::Url, max: u64) -> Result<Vec<u8>, GitHubError> {
        let what = format!("取 {url}");
        let response = self
            .client
            .get(url.clone())
            .send()
            .await
            .map_err(|e| unavailable(&what, &e))?;
        match response.status() {
            StatusCode::OK => read_limited(response, max)
                .await
                .map_err(|e| e.context(&what)),
            StatusCode::NOT_FOUND => Err(GitHubError::NotFound(format!("没有 {url}"))),
            status => Err(GitHubError::Unavailable(format!("{what} 时回了 {status}"))),
        }
    }

    /// 经 API 取文件，给 raw 不通时用。不带令牌时每个出口 IP 每小时 60 次，所以不用它取别的。
    pub async fn file_via_api(
        &self,
        repo: &Repo,
        reference: &str,
        path: &str,
        max: u64,
    ) -> Result<Vec<u8>, GitHubError> {
        let what = format!("经 API 取 {repo} 在 {reference} 里的 {path}");
        let response = self
            .client
            .get(format!(
                "{}/repos/{repo}/contents/{path}?ref={reference}",
                self.api
            ))
            .header("Accept", "application/vnd.github.raw+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .send()
            .await
            .map_err(|e| unavailable(&what, &e))?;
        match response.status() {
            StatusCode::OK => read_limited(response, max)
                .await
                .map_err(|e| e.context(&what)),
            StatusCode::NOT_FOUND => Err(GitHubError::NotFound(format!("没有 {what}"))),
            StatusCode::FORBIDDEN | StatusCode::TOO_MANY_REQUESTS => Err(GitHubError::RateLimited(
                format!("{what}：GitHub 限速了，几分钟后再试"),
            )),
            status => Err(GitHubError::Unavailable(format!(
                "{what}：GitHub 回了 {status}"
            ))),
        }
    }
}

/// 搜索结果里的一个仓库。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    pub repo: Repo,
    pub stars: u32,
    /// 最近一次推送的时间（GitHub 给的原文）：没变就不用重读这个仓库。
    pub pushed_at: String,
}

/// 搜索的一页。
#[derive(Debug, Clone, Default)]
pub struct SearchPage {
    pub repos: Vec<Found>,
    /// 一共有多少个，GitHub 最多给前 1000 个。
    pub total: u32,
}

#[derive(serde::Deserialize)]
struct SearchResponse {
    total_count: u32,
    items: Vec<SearchItem>,
}

#[derive(serde::Deserialize)]
struct SearchItem {
    name: String,
    owner: SearchOwner,
    stargazers_count: u32,
    #[serde(default)]
    pushed_at: Option<String>,
}

#[derive(serde::Deserialize)]
struct SearchOwner {
    login: String,
}

impl GitHub {
    /// 搜索 API 的一页（`page` 从 1 开始，每页 100 个，按星数从多到少）。不带令牌时每个出口 IP 每分钟 10 次，
    /// 超了 GitHub 回 403 或 429，报 [`GitHubError::RateLimited`]。
    pub async fn search(&self, query: &str, page: u32) -> Result<SearchPage, GitHubError> {
        let page = page.to_string();
        let url = reqwest::Url::parse_with_params(
            &format!("{}/search/repositories", self.api),
            [
                ("q", query),
                ("sort", "stars"),
                ("order", "desc"),
                ("per_page", "100"),
                ("page", page.as_str()),
            ],
        )
        .map_err(|e| GitHubError::Unavailable(format!("搜索的地址不对：{e}")))?;
        let response = self
            .client
            .get(url)
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .send()
            .await
            .map_err(|e| unavailable("搜索社区插件", &e))?;
        match response.status() {
            StatusCode::OK => {}
            StatusCode::FORBIDDEN | StatusCode::TOO_MANY_REQUESTS => {
                return Err(GitHubError::RateLimited(
                    "GitHub 限速了，几分钟后再试".to_owned(),
                ));
            }
            status => {
                return Err(GitHubError::Unavailable(format!(
                    "搜索社区插件时 GitHub 回了 {status}"
                )));
            }
        }
        let body = read_limited(response, MAX_REFS_BYTES).await?;
        let found: SearchResponse = serde_json::from_slice(&body)
            .map_err(|e| GitHubError::Unavailable(format!("GitHub 给的搜索结果读不懂：{e}")))?;
        Ok(SearchPage {
            total: found.total_count,
            repos: found
                .items
                .into_iter()
                .map(|item| Found {
                    repo: Repo {
                        owner: item.owner.login,
                        name: item.name,
                    },
                    stars: item.stargazers_count,
                    pushed_at: item.pushed_at.unwrap_or_default(),
                })
                .collect(),
        })
    }
}

impl GitHubError {
    fn context(self, what: &str) -> Self {
        match self {
            Self::Unavailable(m) => Self::Unavailable(format!("{what}：{m}")),
            Self::Timeout(m) => Self::Timeout(format!("{what}：{m}")),
            Self::NotFound(m) => Self::NotFound(format!("{what}：{m}")),
            Self::TooBig(m) => Self::TooBig(format!("{what}：{m}")),
            Self::RateLimited(m) => Self::RateLimited(format!("{what}：{m}")),
        }
    }
}

fn client(proxy: &CoreProxy) -> reqwest::ClientBuilder {
    reqwest::Client::builder()
        .proxy(proxy.for_client())
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(TIMEOUT)
        .user_agent(concat!("wonton-core/", env!("CARGO_PKG_VERSION")))
}

fn unavailable(what: &str, e: &reqwest::Error) -> GitHubError {
    if e.is_timeout() {
        GitHubError::Timeout(format!("{what}：请求超时，没有在时限内答完"))
    } else {
        GitHubError::Unavailable(format!("{what}：连不上 GitHub（{e}）"))
    }
}

async fn read_limited(mut response: reqwest::Response, max: u64) -> Result<Vec<u8>, GitHubError> {
    let too_big = || GitHubError::TooBig(format!("超过 {} KiB", max.div_ceil(1024)));
    if response.content_length().is_some_and(|len| len > max) {
        return Err(too_big());
    }
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|e| unavailable("读响应", &e))?
    {
        body.extend_from_slice(&chunk);
        if body.len() as u64 > max {
            return Err(too_big());
        }
    }
    Ok(body)
}

/// 一行 pkt-line：4 位十六进制的长度（含这 4 位）加内容。GitHub 对长度算错的请求回空响应，不报错。
fn pkt_line(payload: &str) -> String {
    format!("{:04x}{payload}", payload.len() + 4)
}

/// 只要 tag，注解 tag 带上它指向的提交（`peel`）。
fn ls_refs_request() -> String {
    [
        pkt_line("command=ls-refs\n"),
        "0001".to_owned(),
        pkt_line("peel\n"),
        pkt_line("ref-prefix refs/tags/\n"),
        "0000".to_owned(),
    ]
    .concat()
}

/// `ls-refs` 的响应：每行 `<对象> refs/tags/<名字>[ peeled:<提交>]`，以 `0000` 结束。
fn parse_ls_refs(body: &[u8]) -> Result<Vec<Tag>, String> {
    let mut tags = Vec::new();
    let mut rest = body;
    loop {
        let (len, after) = rest.split_at_checked(4).ok_or("响应在结束标记前断了")?;
        let len = std::str::from_utf8(len)
            .ok()
            .and_then(|l| usize::from_str_radix(l, 16).ok())
            .ok_or("pkt-line 的长度不是十六进制")?;
        if len == 0 {
            return Ok(tags);
        }
        let (line, after) = len
            .checked_sub(4)
            .and_then(|n| after.split_at_checked(n))
            .ok_or("pkt-line 的长度不对")?;
        rest = after;
        let line = std::str::from_utf8(line).map_err(|_| "ref 不是 UTF-8")?;
        if let Some(tag) = parse_ref(line.trim_end_matches('\n')) {
            tags.push(tag);
        }
    }
}

fn parse_ref(line: &str) -> Option<Tag> {
    let mut parts = line.split(' ');
    let object = parts.next()?;
    let name = parts.next()?.strip_prefix("refs/tags/")?;
    let peeled = parts.find_map(|attr| attr.strip_prefix("peeled:"));
    let commit = peeled.unwrap_or(object);
    is_commit_id(commit).then(|| Tag {
        name: name.to_owned(),
        commit: commit.to_owned(),
    })
}

/// 40 位小写十六进制：git 的 SHA-1 对象名。
pub fn is_commit_id(text: &str) -> bool {
    text.len() == 40 && text.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

#[cfg(test)]
#[path = "github_http_tests.rs"]
mod http_tests;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ls_refs_requests_and_responses_are_pkt_lines() {
        assert_eq!(
            ls_refs_request(),
            "0014command=ls-refs\n00010009peel\n001aref-prefix refs/tags/\n0000"
        );
        let a = "a".repeat(40);
        let b = "b".repeat(40);
        let body = [
            pkt_line(&format!("{a} refs/tags/v1.0.0\n")),
            pkt_line(&format!("{a} refs/tags/v1.1.0 peeled:{b}\n")),
            "0000".to_owned(),
        ]
        .concat();
        assert_eq!(
            parse_ls_refs(body.as_bytes()).unwrap(),
            [
                Tag {
                    name: "v1.0.0".into(),
                    commit: a.clone()
                },
                Tag {
                    name: "v1.1.0".into(),
                    commit: b
                },
            ]
        );
        assert!(parse_ls_refs(pkt_line(&format!("{a} refs/tags/v1\n")).as_bytes()).is_err());
    }
}
