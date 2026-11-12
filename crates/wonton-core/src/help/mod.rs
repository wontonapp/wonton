//! 帮助文档（ADR-0034）：`wonton__help` 工具读的内容。镜像里打进一份，官方索引的 `docs` 指向更新的提交时取来核对，全部通过后才切换。
//!
//! 取新版在后台进行，工具最多等 [`WAIT`]：家里主机上不了外网时也要马上用手头的文档回答。

mod page;
mod sync;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use futures_util::future::BoxFuture;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use wonton_protocol::StoreFailure;

use wonton_l10n::Arg;

use crate::l10n;
use crate::plugin::PluginManager;
use crate::plugin::languages::Languages;
use crate::plugin::manager::now_secs;
use crate::plugin::store::read_json;
use page::{Page, Pages};

/// 镜像里那份文档的位置，`WONTON_DOCS` 能换。
pub const DEFAULT_DOCS: &str = "/usr/share/wonton/docs";
/// 工具等取新版最多这么久，没取完的接着在后台取，下一次调用用上。
const WAIT: Duration = Duration::from_secs(3);
/// 这个提交取失败后，隔这么久才再试：每次调用都试的话，断网时每次都白等。
const SYNC_RETRY: Duration = Duration::from_secs(60);
/// 记着这份文档是哪个提交、什么时候的：镜像里和缓存目录里各有一个。
const SOURCE_FILE: &str = "source.json";
const CURRENT_FILE: &str = "current.json";

/// 这台核心的版本号，和升级接口报的是同一个。开发构建没有版本号，为空。
pub type CoreVersion = Arc<dyn Fn() -> BoxFuture<'static, Option<String>> + Send + Sync>;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Source {
    commit: String,
    /// 镜像里那份是提交的时间，取来的是取到的时间。
    updated_at_ms: u64,
    #[serde(default)]
    min_core: Option<String>,
}

#[derive(Debug)]
struct Current {
    source: Source,
    dir: PathBuf,
    pages: Pages,
}

#[derive(Default)]
struct Attempt {
    failed: Option<(String, Instant)>,
}

pub struct Help {
    plugins: Arc<PluginManager>,
    version: CoreVersion,
    baseline: PathBuf,
    cache: PathBuf,
    current: RwLock<Option<Arc<Current>>>,
    /// 最近一次取新版没成功的原因。
    problem: Mutex<Option<String>>,
    /// 同时只取一次。
    attempt: tokio::sync::Mutex<Attempt>,
}

impl Help {
    pub fn new(baseline: PathBuf, plugins: Arc<PluginManager>, version: CoreVersion) -> Arc<Self> {
        let cache = plugins.docs_cache();
        Arc::new(Self {
            plugins,
            version,
            baseline,
            cache,
            current: RwLock::default(),
            problem: Mutex::default(),
            attempt: tokio::sync::Mutex::default(),
        })
    }

    /// 工具的结果：没写 `topic` 时是目录，写了是那一页。文档一份都没有、主题不存在时报错，错误信息给 Agent 看。
    pub async fn answer(
        self: &Arc<Self>,
        topic: Option<&str>,
        lang: Option<&str>,
    ) -> Result<Value, String> {
        let languages = lang.map_or_else(
            || self.plugins.owner_languages(),
            |lang| Languages::from_header(Some(lang)),
        );
        let this = self.clone();
        let refresh = tokio::spawn(async move { this.refresh().await });
        // 超时只是不再等，取新版接着在后台做完。
        let _ = tokio::time::timeout(WAIT, refresh).await;
        let version = (self.version)().await;
        let current = self.loaded(version.as_deref()).ok_or_else(|| {
            format!(
                "docs_unavailable: {}",
                l10n::text(
                    &languages,
                    "chat-help-docs-unavailable",
                    &[("dir", self.baseline.display().to_string().into())],
                )
            )
        })?;
        let mut result = json!({
            "core_version": version.as_deref().unwrap_or("unknown"),
            "docs_commit": current.source.commit,
            "docs_updated_days_ago": days_ago(current.source.updated_at_ms),
        });
        let problem = self.problem.lock().expect("帮助文档的锁中毒").clone();
        if let Some(problem) = problem {
            let days = days_ago(current.source.updated_at_ms);
            // 整段用主人的语言：取新版是后台发起的，`problem` 是当时按主人的语言写好的，
            // 后缀也用主人的语言，别和 Agent 在 `lang` 里要的文档语言混着。
            let owner = self.plugins.owner_languages();
            result["refresh_problem"] = json!(l10n::text(
                &owner,
                "chat-help-docs-age",
                &[("problem", problem.into()), ("days", Arg::from(days))],
            ));
        }
        match topic {
            None => result["topics"] = directory(&current.pages, &languages),
            Some(topic) => page_result(&mut result, &current.pages, topic, &languages, &version)?,
        }
        Ok(result)
    }

    /// 看官方索引有没有更新的文档，有就取。原因记进 `problem`，这里不报错：手头的文档照常能答。
    /// 原因用主人的语言写：取新版是后台自己发起的，没有请求头。
    async fn refresh(&self) {
        let langs = self.plugins.owner_languages();
        let mut attempt = self.attempt.lock().await;
        let version = (self.version)().await;
        self.loaded(version.as_deref());
        let (index, problem) = self.plugins.official_index().await;
        let reason = problem
            .filter(|p| !matches!(p.reason, StoreFailure::Incomplete | StoreFailure::Disabled))
            .map(|p| {
                l10n::text(
                    &langs,
                    "chat-help-refresh-failed",
                    &[("details", p.details.into())],
                )
            })
            .or(index.docs_problem.clone());
        let Some(spec) = index.docs else {
            self.set_problem(reason);
            return;
        };
        let current = self.current.read().expect("帮助文档的锁中毒").clone();
        if current
            .as_ref()
            .is_some_and(|c| c.source.commit == spec.commit)
        {
            attempt.failed = None;
            self.set_problem(reason);
            return;
        }
        if older_than(version.as_deref(), &spec.min_core) {
            self.set_problem(Some(l10n::text(
                &langs,
                "chat-help-refresh-needs-core",
                &[
                    ("required", spec.min_core.clone().into()),
                    (
                        "core",
                        version.as_deref().unwrap_or("unknown").to_owned().into(),
                    ),
                ],
            )));
            return;
        }
        if let Some((commit, at)) = &attempt.failed
            && commit == &spec.commit
            && at.elapsed() < SYNC_RETRY
        {
            return;
        }
        match sync::fetch(
            &spec,
            &self.plugins.official_files(),
            &self.cache,
            current.as_deref().map(|c| c.dir.as_path()),
            &self.plugins.owner_languages(),
        )
        .await
        {
            Ok(next) => {
                attempt.failed = None;
                *self.current.write().expect("帮助文档的锁中毒") = Some(Arc::new(next));
                self.set_problem(reason);
            }
            Err(error) => {
                tracing::warn!(commit = %spec.commit, %error, "取不到新的帮助文档，旧的照常用");
                attempt.failed = Some((spec.commit.clone(), Instant::now()));
                self.set_problem(Some(l10n::text(
                    &langs,
                    "chat-help-refresh-failed",
                    &[("details", error.to_string().into())],
                )));
            }
        }
    }

    fn set_problem(&self, problem: Option<String>) {
        *self.problem.lock().expect("帮助文档的锁中毒") = problem;
    }

    /// 手头的文档。第一次用时读磁盘：镜像里那份和取来的那份，哪个新用哪个；取来的那份要的核心比这台新时不用，回滚核心
    /// 后就是这样。
    fn loaded(&self, version: Option<&str>) -> Option<Arc<Current>> {
        let mut current = self.current.write().expect("帮助文档的锁中毒");
        if current.is_none() {
            let baseline = load(&self.baseline, &self.baseline.join(SOURCE_FILE));
            let cached = load_cached(&self.cache).filter(|c| {
                !c.source
                    .min_core
                    .as_deref()
                    .is_some_and(|m| older_than(version, m))
            });
            let newest = [baseline, cached]
                .into_iter()
                .flatten()
                .max_by_key(|c| c.source.updated_at_ms);
            *current = newest.map(Arc::new);
        }
        current.clone()
    }
}

fn load(dir: &Path, source: &Path) -> Option<Current> {
    let source: Source = match read_json(source) {
        Ok(Some(source)) => source,
        Ok(None) => return None,
        Err(error) => {
            tracing::error!(%error, "帮助文档的来源读不了");
            return None;
        }
    };
    match page::load_dir(dir) {
        Ok(pages) if !pages.is_empty() => Some(Current {
            source,
            dir: dir.to_owned(),
            pages,
        }),
        Ok(_) => {
            tracing::warn!(dir = %dir.display(), "帮助文档是空的");
            None
        }
        Err(error) => {
            tracing::error!(dir = %dir.display(), %error, "帮助文档读不了");
            None
        }
    }
}

fn load_cached(cache: &Path) -> Option<Current> {
    let current = cache.join(CURRENT_FILE);
    let source: Source = read_json(&current).ok().flatten()?;
    let dir = cache.join(&source.commit);
    let pages = page::load_dir(&dir).ok().filter(|p| !p.is_empty())?;
    Some(Current { source, dir, pages })
}

fn days_ago(at_ms: u64) -> u64 {
    now_secs().saturating_sub(at_ms / 1000) / 86_400
}

/// 版本都读得懂、而且 `version` 更旧。开发构建没有版本号，不比较。
fn older_than(version: Option<&str>, required: &str) -> bool {
    let parse = |v: &str| semver::Version::parse(v.trim_start_matches('v')).ok();
    match (version.and_then(parse), parse(required)) {
        (Some(version), Some(required)) => version < required,
        _ => false,
    }
}

fn chosen<'p>(pages: &'p Pages, topic: &str, languages: &Languages) -> Option<&'p Page> {
    languages.choose(pages.get(topic)?)
}

fn directory(pages: &Pages, languages: &Languages) -> Value {
    let topics: Vec<Value> = pages
        .keys()
        .filter_map(|topic| {
            let page = chosen(pages, topic, languages)?;
            let mut row = json!({
                "topic": topic,
                "lang": page.lang,
                "title": page.title,
                "summary": page.summary,
            });
            add_requirements(&mut row, page);
            Some(row)
        })
        .collect();
    Value::Array(topics)
}

fn add_requirements(value: &mut Value, page: &Page) {
    if let Some(required) = &page.requires_core {
        value["requires_core"] = json!(required);
    }
    if let Some(required) = &page.requires_app {
        value["requires_app"] = json!(required);
    }
}

fn page_result(
    result: &mut Value,
    pages: &Pages,
    topic: &str,
    languages: &Languages,
    version: &Option<String>,
) -> Result<(), String> {
    let Some(page) = chosen(pages, topic, languages) else {
        let topics: Vec<&str> = pages.keys().map(String::as_str).collect();
        return Err(format!(
            "invalid_input: {}",
            l10n::text(
                languages,
                "chat-help-topic-missing",
                &[
                    ("topic", topic.into()),
                    ("topics", l10n::list(languages, &topics).into()),
                ],
            )
        ));
    };
    result["topic"] = json!(topic);
    result["lang"] = json!(page.lang);
    result["title"] = json!(page.title);
    result["summary"] = json!(page.summary);
    add_requirements(result, page);
    if let Some(required) = &page.requires_core
        && older_than(version.as_deref(), required)
    {
        result["note"] = json!(l10n::text(
            languages,
            "chat-help-page-needs-core",
            &[
                ("required", required.as_str().into()),
                (
                    "core",
                    version.as_deref().unwrap_or("unknown").to_owned().into()
                ),
            ],
        ));
    }
    result["body"] = json!(page.body);
    Ok(())
}
