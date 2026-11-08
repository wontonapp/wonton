//! 房间里说话用的语言（ADR-0043）：问核心的 `wonton__user_locale`，按人缓存几分钟。
//!
//! 取不到时一级一级退：房间主人的语言 → Bot 主人的语言 → 核心的默认语言（英文）。
//! 房间主人就是 Bot 主人时跳过第二级。每退一级打一条 `warn`，同一个人同一原因在一个缓存
//! 周期里只打一次（查询结果连失败一起缓存，缓存过期才会再问核心）。

use std::{
    collections::HashMap,
    future::Future,
    time::{Duration, Instant},
};

use rmcp::{
    ClientLifecycleMode, ClientServiceExt,
    model::{
        CallToolRequestParams, CallToolResult, ClientCapabilities, ClientConfig, Implementation,
        ProtocolVersion,
    },
    transport::StreamableHttpClientTransport,
};
use serde_json::{Map, Value, json};
use tokio::sync::Mutex;
use wonton_l10n::Lang;
use wonton_matrix::{OwnedUserId, UserId};

use crate::l10n::{CATALOG, default_lang};

/// 缓存有效期：主人换了语言，过几分钟就跟上。
const CACHE_TTL: Duration = Duration::from_secs(300);
/// 一次查询连连接带调用最多等多久。
const LOOKUP_TIMEOUT: Duration = Duration::from_secs(10);
/// 核心的 MCP 工具都带这个前缀。
const USER_LOCALE_TOOL: &str = "wonton__user_locale";

/// 查不到的原因类别：进日志，也决定往哪一级退。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Failure {
    /// `locale` 为 null、`languages` 为空：这个人还没有语言记录。
    NoRecord,
    PermissionDenied,
    NotFound,
    NotThisServer,
    /// 传输失败和拿不到结果的其他错误。
    Transport,
}

impl Failure {
    pub fn as_str(self) -> &'static str {
        match self {
            Failure::NoRecord => "locale_null",
            Failure::PermissionDenied => "permission_denied",
            Failure::NotFound => "not_found",
            Failure::NotThisServer => "not_this_server",
            Failure::Transport => "transport",
        }
    }
}

pub type Lookup = Result<Vec<String>, Failure>;

/// 查一个人语言的地方：生产走核心 MCP，测试用内存替身。
pub trait LocaleSource {
    fn user_locale(&self, user_id: &str) -> impl Future<Output = Lookup> + Send;
}

struct Cached {
    expires: Instant,
    entry: Lookup,
}

/// 按人缓存的语言查询和三级兜底。
pub struct Locales<S> {
    source: S,
    bot_owner: OwnedUserId,
    ttl: Duration,
    cache: Mutex<HashMap<OwnedUserId, Cached>>,
}

impl<S: LocaleSource> Locales<S> {
    pub fn new(source: S, bot_owner: OwnedUserId) -> Self {
        Self::build(source, bot_owner, CACHE_TTL)
    }

    #[cfg(test)]
    pub fn with_ttl(source: S, bot_owner: OwnedUserId, ttl: Duration) -> Self {
        Self::build(source, bot_owner, ttl)
    }

    fn build(source: S, bot_owner: OwnedUserId, ttl: Duration) -> Self {
        Self {
            source,
            bot_owner,
            ttl,
            cache: Mutex::new(HashMap::new()),
        }
    }

    /// 房间说话用的语言：房间主人取不到就退到 Bot 主人，再取不到用英文。
    pub async fn lang(&self, room_owner: &UserId) -> Lang {
        if room_owner == &*self.bot_owner {
            let (entry, fresh) = self.lookup(room_owner).await;
            return match normalize(entry) {
                Ok(languages) => CATALOG.negotiate(&languages),
                Err(failure) => {
                    if fresh {
                        warn_fallback(room_owner, "english", failure);
                    }
                    default_lang()
                }
            };
        }
        let (entry, fresh) = self.lookup(room_owner).await;
        match normalize(entry) {
            Ok(languages) => CATALOG.negotiate(&languages),
            Err(failure) => {
                if fresh {
                    warn_fallback(room_owner, "bot_owner", failure);
                }
                let (entry, fresh) = self.lookup(&self.bot_owner).await;
                match normalize(entry) {
                    Ok(languages) => CATALOG.negotiate(&languages),
                    Err(failure) => {
                        if fresh {
                            warn_fallback(&self.bot_owner, "english", failure);
                        }
                        default_lang()
                    }
                }
            }
        }
    }

    /// 查一个人。第二个返回值表示这次真的问了核心：只有这才打 warn，用缓存的重复消息不打。
    async fn lookup(&self, user: &UserId) -> (Lookup, bool) {
        {
            let cache = self.cache.lock().await;
            if let Some(cached) = cache
                .get(user)
                .filter(|cached| cached.expires > Instant::now())
            {
                return (cached.entry.clone(), false);
            }
        }
        let entry = self.source.user_locale(user.as_str()).await;
        self.cache.lock().await.insert(
            user.to_owned(),
            Cached {
                expires: Instant::now() + self.ttl,
                entry: entry.clone(),
            },
        );
        (entry, true)
    }
}

fn warn_fallback(user: &UserId, fallback: &str, failure: Failure) {
    tracing::warn!(
        user = %user,
        reason = failure.as_str(),
        fallback,
        "no language for this person; falling back one level"
    );
}

/// 成功但没有语言（`locale` 为 null）按“没有记录”算。
fn normalize(entry: Lookup) -> Lookup {
    match entry {
        Ok(languages) if languages.is_empty() => Err(Failure::NoRecord),
        other => other,
    }
}

/// 核心 MCP 的客户端。每次查询新开一个 Streamable HTTP 会话，查完关掉（DELETE），
/// 不跨长时间保留会话状态。
pub struct CoreMcp {
    url: String,
}

impl CoreMcp {
    pub fn new(mcp_url: &str) -> Self {
        Self {
            url: mcp_url.to_owned(),
        }
    }

    async fn call(&self, user_id: &str) -> Result<Lookup, ()> {
        let transport = StreamableHttpClientTransport::from_uri(self.url.as_str());
        let client = ClientConfig::new(
            ClientCapabilities::default(),
            Implementation::new("wonton-acp-bridge", env!("CARGO_PKG_VERSION")),
        )
        .serve_with_lifecycle(
            transport,
            ClientLifecycleMode::Auto {
                preferred_versions: vec![ProtocolVersion::V_2026_07_28],
                legacy_version: Some(ProtocolVersion::V_2025_11_25),
            },
        )
        .await
        .map_err(|_| ())?;
        let result = client
            .peer()
            .call_tool(
                CallToolRequestParams::new(USER_LOCALE_TOOL)
                    .with_arguments(Map::from_iter([("user_id".to_owned(), json!(user_id))])),
            )
            .await
            .map_err(|_| ())?;
        let entry = parse_result(&result);
        // 会话在关闭时由传输层 DELETE 掉。
        client.cancellation_token().cancel();
        Ok(entry)
    }
}

/// 语言从哪来：生产问核心，测试固定回一种。`Locales` 的泛型参数要具体类型，用它包一层。
pub enum Origin {
    Core(CoreMcp),
    /// 固定回简体中文，测试里的断言照中文写。
    #[cfg(test)]
    Fixed,
}

impl Origin {
    pub fn core(mcp_url: &str) -> Self {
        Self::Core(CoreMcp::new(mcp_url))
    }
}

impl LocaleSource for Origin {
    async fn user_locale(&self, user_id: &str) -> Lookup {
        match self {
            Self::Core(source) => source.user_locale(user_id).await,
            #[cfg(test)]
            Self::Fixed => Ok(vec!["zh-Hans".to_owned()]),
        }
    }
}

impl LocaleSource for CoreMcp {
    async fn user_locale(&self, user_id: &str) -> Lookup {
        match tokio::time::timeout(LOOKUP_TIMEOUT, self.call(user_id)).await {
            Ok(Ok(entry)) => entry,
            Ok(Err(())) | Err(_) => Err(Failure::Transport),
        }
    }
}

/// 工具结果：工具自己的错误（`permission_denied: …` 这类前缀）按类别读，成功时读 `languages`。
fn parse_result(result: &CallToolResult) -> Lookup {
    let text = result
        .content
        .iter()
        .find_map(|block| block.as_text().map(|text| text.text.as_str()))
        .unwrap_or_default();
    if result.is_error == Some(true) {
        return Err(failure(text));
    }
    let Ok(value) = serde_json::from_str::<Value>(text) else {
        return Err(Failure::Transport);
    };
    Ok(["languages", "locale"]
        .iter()
        .find_map(|field| match &value[*field] {
            Value::Array(items) => Some(
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(ToOwned::to_owned)
                    .collect::<Vec<_>>(),
            ),
            Value::String(tag) => Some(vec![tag.clone()]),
            _ => None,
        })
        .unwrap_or_default())
}

/// 工具错误文字的错误类别：第一个分隔符前的机器可读前缀。
fn failure(text: &str) -> Failure {
    match text.split(':').next().unwrap_or_default().trim() {
        "permission_denied" => Failure::PermissionDenied,
        "not_found" => Failure::NotFound,
        "not_this_server" => Failure::NotThisServer,
        // `locale` 为 null 时核心给成功结果，不进这里；其余（invalid_input、上游异常）算查询失败。
        _ => Failure::Transport,
    }
}

#[cfg(test)]
#[path = "locale_tests.rs"]
mod tests;
