use std::{
    collections::HashMap,
    io,
    sync::{Arc, Mutex},
    time::Duration,
};

use rmcp::{
    ErrorData,
    handler::server::ServerHandler,
    model::{
        CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, Implementation,
        JsonObject, ListToolsResult, PaginatedRequestParams, ServerCapabilities, ServerConfig,
        Tool,
    },
    service::{RequestContext, RoleServer},
    transport::streamable_http_server::{
        StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
    },
};
use serde_json::json;
use tracing_subscriber::fmt::MakeWriter;
use wonton_matrix::OwnedUserId;

use super::*;
use crate::room_owner::room_owner_id;

const ALICE: &str = "@alice:example.org";
const BOB: &str = "@bob:example.org";

fn user(id: &str) -> OwnedUserId {
    id.parse().unwrap()
}

fn tags(lang: Lang) -> &'static str {
    CATALOG.tag(lang)
}

/// 按人回结果的替身，记下每次查的是谁。
#[derive(Default)]
struct FakeSource {
    results: Mutex<HashMap<String, Lookup>>,
    calls: Mutex<Vec<String>>,
}

impl FakeSource {
    fn with(entries: &[(&str, Lookup)]) -> Self {
        Self {
            results: Mutex::new(
                entries
                    .iter()
                    .map(|(user, entry)| ((*user).to_owned(), entry.clone()))
                    .collect(),
            ),
            calls: Mutex::new(Vec::new()),
        }
    }

    fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }
}

impl LocaleSource for FakeSource {
    async fn user_locale(&self, user_id: &str) -> Lookup {
        self.calls.lock().unwrap().push(user_id.to_owned());
        self.results
            .lock()
            .unwrap()
            .get(user_id)
            .cloned()
            .unwrap_or(Err(Failure::NotFound))
    }
}

/// 抓 `warn` 日志：断言只看带自己这个人的行。
///
/// 每条测试装自己的线程内订阅器（`set_default`），返回的 guard 要活到断言结束：`downtime_real`
/// 的 testkit 会把全局订阅器装成写 stderr 的（wonton-matrix testkit 的 `init_test_logging`），
/// 后装全局的 `set_global_default` 静默失败、自己的缓冲恒空——全量带 `--include-ignored` 跑时
/// 那 3 条日志断言就是这么红的。线程内订阅器不参与全局竞争，`tokio::test`（current_thread）把
/// 整个测试体跑在装 guard 的线程上，warn 全落进自己的缓冲。
///
/// 装完调一次 `rebuild_interest_cache()`：订阅器的增删会触发全表兴趣重建，重建与别的线程上
/// callsite 懒注册并发时（tracing-core 的无锁链表），后注册的 callsite 可能缓存成
/// `Interest::never` 并把日志全丢，显式重建一遍把毒化的修掉。
#[derive(Clone, Default)]
struct Logs(Arc<Mutex<Vec<u8>>>);

impl io::Write for Logs {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// 一行日志的整次写入都攥着锁，并行测试不会把它拆散。
struct Locked<'a>(std::sync::MutexGuard<'a, Vec<u8>>);

impl io::Write for Locked<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<'a> MakeWriter<'a> for Logs {
    type Writer = Locked<'a>;
    fn make_writer(&'a self) -> Self::Writer {
        Locked(self.0.lock().unwrap())
    }
}

fn logs() -> (Logs, tracing::subscriber::DefaultGuard) {
    let logs = Logs::default();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(logs.clone())
        .with_max_level(tracing::Level::WARN)
        .without_time()
        .finish();
    let guard = tracing::subscriber::set_default(subscriber);
    tracing_core::callsite::rebuild_interest_cache();
    (logs, guard)
}

/// 日志里带某个人（`user=@…`）的行。
impl Logs {
    fn lines_with(&self, marker: &str) -> Vec<String> {
        let text = String::from_utf8(self.0.lock().unwrap().clone()).unwrap();
        text.lines()
            .filter(|line| line.contains(marker))
            .map(ToOwned::to_owned)
            .collect()
    }
}

#[tokio::test]
async fn the_room_owner_s_language_wins() {
    let source = FakeSource::with(&[(ALICE, Ok(vec!["zh-Hant-TW".into()]))]);
    let locales = Locales::new(source, user(BOB));
    assert_eq!(tags(locales.lang(&user(ALICE)).await), "zh-Hant");
    assert_eq!(locales.source.calls(), [ALICE]);
}

#[tokio::test]
async fn a_denied_room_owner_falls_back_to_the_bot_owner_and_warns() {
    let (logs, _guard) = logs();
    let room_owner = user("@denied:example.org");
    let source = FakeSource::with(&[
        ("@denied:example.org", Err(Failure::PermissionDenied)),
        (BOB, Ok(vec!["ja".into()])),
    ]);
    let locales = Locales::new(source, user(BOB));
    assert_eq!(tags(locales.lang(&room_owner).await), "ja");
    // 先按房间主人查，退到 Bot 主人再查一次。
    assert_eq!(locales.source.calls(), ["@denied:example.org", BOB]);
    let logged = logs.lines_with("@denied:example.org");
    assert_eq!(logged.len(), 1, "{logged:?}");
    assert!(logged[0].contains("permission_denied"), "{logged:?}");
    assert!(logged[0].contains("bot_owner"), "{logged:?}");
}

#[tokio::test]
async fn when_both_have_no_language_the_core_default_is_used() {
    let (logs, _guard) = logs();
    let room_owner = user("@unrecorded:example.org");
    let bot_owner = user("@offline:example.org");
    let source = FakeSource::with(&[
        ("@unrecorded:example.org", Err(Failure::NoRecord)),
        ("@offline:example.org", Err(Failure::Transport)),
    ]);
    let locales = Locales::new(source, bot_owner);
    assert_eq!(tags(locales.lang(&room_owner).await), "en");
    // 一级一条 warn：先写房间主人退到 Bot 主人，再写 Bot 主人退到英文。
    let first = logs.lines_with("@unrecorded:example.org");
    assert_eq!(first.len(), 1, "{first:?}");
    assert!(first[0].contains("locale_null"), "{first:?}");
    assert!(first[0].contains("bot_owner"), "{first:?}");
    let second = logs.lines_with("@offline:example.org");
    assert_eq!(second.len(), 1, "{second:?}");
    assert!(second[0].contains("transport"), "{second:?}");
    assert!(second[0].contains("english"), "{second:?}");
}

#[tokio::test]
async fn the_bot_owner_is_queried_once_when_he_is_the_room_owner() {
    let source = FakeSource::with(&[(BOB, Err(Failure::NoRecord))]);
    let locales = Locales::new(source, user(BOB));
    assert_eq!(tags(locales.lang(&user(BOB)).await), "en");
    assert_eq!(locales.source.calls(), [BOB]);
}

#[tokio::test]
async fn one_person_is_queried_once_within_the_cache_ttl() {
    let (logs, _guard) = logs();
    let source = FakeSource::with(&[
        ("@cached:example.org", Ok(vec!["ja".into()])),
        (BOB, Err(Failure::NoRecord)),
    ]);
    let locales = Locales::new(source, user(BOB));
    for _ in 0..3 {
        assert_eq!(tags(locales.lang(&user("@cached:example.org")).await), "ja");
        // 房间主人成功，不退级；Bot 主人一次都不查。
        assert_eq!(locales.source.calls(), ["@cached:example.org"]);
    }
    assert!(logs.lines_with("@cached:example.org").is_empty());

    // 房间主人一直取不到时，warn 每个缓存周期只打一次。
    let source = FakeSource::with(&[
        ("@stale:example.org", Err(Failure::PermissionDenied)),
        (BOB, Ok(vec!["en".into()])),
    ]);
    let locales = Locales::new(source, user(BOB));
    for _ in 0..3 {
        assert_eq!(tags(locales.lang(&user("@stale:example.org")).await), "en");
    }
    assert_eq!(locales.source.calls(), ["@stale:example.org", BOB]);
    assert_eq!(logs.lines_with("@stale:example.org").len(), 1);
}

#[tokio::test]
async fn an_expired_entry_is_queried_again() {
    let source = FakeSource::with(&[(ALICE, Ok(vec!["ja".into()]))]);
    let locales = Locales::with_ttl(source, user(BOB), Duration::ZERO);
    locales.lang(&user(ALICE)).await;
    locales.lang(&user(ALICE)).await;
    assert_eq!(locales.source.calls(), [ALICE, ALICE]);
}

#[test]
fn room_owners_come_from_the_create_event_or_fall_back() {
    let bot = user("@bot:example.org");
    let owner = user(BOB);
    let created = |sender: &str| json!({"type": "m.room.create", "sender": sender, "content": {}});
    // 本服务器的家人建了房间：就是他。
    assert_eq!(
        room_owner_id(Some(&created(ALICE)), &bot, &owner),
        user(ALICE)
    );
    // Bot 自己建的房间、别的服务器的人建的房间、取不到：都按 Bot 的主人。
    assert_eq!(
        room_owner_id(Some(&created("@bot:example.org")), &bot, &owner),
        owner
    );
    assert_eq!(
        room_owner_id(Some(&created("@someone:elsewhere.org")), &bot, &owner),
        owner
    );
    assert_eq!(room_owner_id(None, &bot, &owner), owner);
}

struct LocaleServer;

impl ServerHandler for LocaleServer {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("fake-core", "0.0.0"))
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _ctx: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        Ok(ListToolsResult::with_all_items(vec![Tool::new(
            USER_LOCALE_TOOL.to_owned(),
            "fake user locale",
            JsonObject::default(),
        )]))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        let user = request
            .arguments
            .as_ref()
            .and_then(|arguments| arguments.get("user_id"))
            .and_then(|value| value.as_str())
            .unwrap_or_default()
            .to_owned();
        let result = match user.as_str() {
            ALICE => CallToolResult::success(vec![ContentBlock::text(
                json!({"user_id": ALICE, "locale": "zh-Hant-TW", "languages": ["zh-Hant-TW"]})
                    .to_string(),
            )]),
            BOB => CallToolResult::error(vec![ContentBlock::text(
                "permission_denied: reading someone else's languages needs the owner.identity permission",
            )]),
            "@nobody:example.org" => CallToolResult::success(vec![ContentBlock::text(
                json!({"user_id": "@nobody:example.org", "locale": null, "languages": []})
                    .to_string(),
            )]),
            _ => CallToolResult::success(vec![ContentBlock::text(
                json!({"user_id": user, "locale": "fr-FR", "languages": ["fr-FR"]}).to_string(),
            )]),
        };
        Ok(result.into())
    }
}

/// 起一个真的 Streamable HTTP MCP 服务，返回客户端和它的地址。
async fn mcp_server() -> (CoreMcp, tokio::task::JoinHandle<()>) {
    let service = StreamableHttpService::new(
        || Ok(LocaleServer),
        LocalSessionManager::default().into(),
        StreamableHttpServerConfig::default(),
    );
    let router = axum::Router::new().nest_service("/mcp", service);
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });
    (CoreMcp::new(&format!("http://{address}/mcp")), task)
}

#[tokio::test]
async fn the_mcp_client_reads_languages_and_error_categories() {
    let (client, server) = mcp_server().await;
    assert_eq!(
        client.user_locale(ALICE).await,
        Ok(vec!["zh-Hant-TW".to_owned()])
    );
    assert!(matches!(
        client.user_locale(BOB).await,
        Err(Failure::PermissionDenied)
    ));
    assert_eq!(
        client.user_locale("@nobody:example.org").await,
        Ok(Vec::new())
    );
    assert!(matches!(
        client.user_locale("@stranger:elsewhere.org").await,
        Ok(languages) if languages == ["fr-FR"]
    ));
    server.abort();
}

#[tokio::test]
async fn an_unreachable_core_counts_as_a_transport_failure() {
    // 绑了又放的端口不会再有人听。
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    let client = CoreMcp::new(&format!("http://{address}/mcp"));
    assert!(matches!(
        client.user_locale(ALICE).await,
        Err(Failure::Transport)
    ));
}

#[test]
fn error_prefixes_map_to_categories() {
    assert_eq!(
        failure("permission_denied: no owner.identity"),
        Failure::PermissionDenied
    );
    assert_eq!(failure("not_found: no person"), Failure::NotFound);
    assert_eq!(
        failure("not_this_server: @a:elsewhere.org"),
        Failure::NotThisServer
    );
    assert_eq!(failure("invalid_input: ?"), Failure::Transport);
    assert_eq!(failure("something new"), Failure::Transport);
    assert_eq!(failure(""), Failure::Transport);
}
