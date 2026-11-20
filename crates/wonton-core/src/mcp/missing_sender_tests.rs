//! 有 Bot 记录的调用方说不出替谁办事（没带 `X-Wonton-On-Behalf-Of`，也匹配不到进行中的轮次）时按最小权限：
//! AI 动手封顶只读，会改、会删的工具按权限缺失拒绝。按 [`super::server`] 的顺序先认说话人、再算封顶。

use hyper::HeaderMap;
use hyper::header::HeaderName;
use serde_json::{Value, json};
use wonton_protocol::{AiLevel, Answer, BotKeeper, BotRef, ConfirmationCategory};

use super::ai_level_tests::{attach_ledger, caller, member, names, notes, role, start_turn};
use super::server::turn_sender;
use super::test_setup::Setup;
use super::tests::{failed, text};
use super::tool_failure_tests::failure_kind;
use super::{Caller, TurnSender};
use crate::plugin::PluginId;

const AGENT: &str = "com.example.agent";
const SEARCH: &str = "app-example-notes__search";
const ARCHIVE: &str = "app-example-notes__archive";
const SAVE: &str = "wonton__save_file";
const SEND: &str = "wonton__send_message";
const ALICE: &str = "@alice:wonton.localhost";

/// 和 MCP 服务器处理每个请求一样：按请求头和轮次认说话人，再算这次的封顶。
fn resolved(s: &Setup, caller: &Caller, headers: Option<&HeaderMap>) -> Caller {
    let mut caller = caller.clone();
    caller.sender = turn_sender(&s.bus, &caller, headers);
    caller.ai_level = Some(s.bus.ai_cap(&caller));
    caller
}

fn on_behalf_of(value: &str) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(
        HeaderName::from_static("x-wonton-on-behalf-of"),
        value.parse().unwrap(),
    );
    headers
}

fn private_bot() -> Caller {
    caller(
        "alice-bot",
        BotKeeper::User {
            username: "alice".into(),
        },
    )
}

fn household() -> Caller {
    caller("household", BotKeeper::Household)
}

/// 家里只有 alice 一个成员，她的角色是“改动自己确认”、能用共享 Bot：按归属人或按能用共享 Bot 的人算，都会放开。
async fn home() -> Setup {
    let s = notes().await;
    let self_confirms = role(&s, "自己确认", AiLevel::SelfConfirms, true);
    member(&s, "alice", &self_confirms);
    s
}

/// 只读：列不到会改、会删的工具，调了按权限缺失拒绝、不发卡片；只读工具照常能用。
async fn assert_least_privilege(s: &Setup, me: &Caller) {
    assert_eq!(me.ai_level, Some(AiLevel::ReadOnly));
    let listed = names(s, me).await;
    assert!(listed.contains(&SEARCH.to_owned()), "{listed:?}");
    assert!(!listed.contains(&ARCHIVE.to_owned()), "{listed:?}");
    assert!(!listed.contains(&SAVE.to_owned()), "{listed:?}");
    let read = s.call_as(me, SEARCH, json!({"name": "a"})).await;
    assert_eq!(text(&read), "找到 a.txt");
    let denied = s.call_as(me, ARCHIVE, json!({"name": "a"})).await;
    assert_eq!(failure_kind(&denied), Some("permission_denied"));
    let denied = s
        .call_as(me, SAVE, json!({"files": ["a.mp3"], "share": "music"}))
        .await;
    assert_eq!(failure_kind(&denied), Some("permission_denied"));
    assert!(s.cards.posted.lock().unwrap().is_empty());
}

#[tokio::test]
async fn private_bot_without_header_or_live_turn_is_read_only() {
    let s = home().await;
    attach_ledger(&s);
    let me = resolved(&s, &private_bot(), None);
    assert_eq!(me.sender, None);
    assert_least_privilege(&s, &me).await;
}

#[tokio::test]
async fn shared_bot_without_header_or_live_turn_is_read_only() {
    let s = home().await;
    attach_ledger(&s);
    let me = resolved(&s, &household(), None);
    assert_eq!(me.sender, None);
    assert_least_privilege(&s, &me).await;
}

#[tokio::test]
async fn header_naming_someone_the_bot_does_not_listen_to_is_read_only() {
    let s = home().await;
    for bot in [private_bot(), household()] {
        let me = resolved(&s, &bot, Some(&on_behalf_of("@stranger:wonton.localhost")));
        assert_eq!(me.sender, Some(TurnSender::Unknown));
        assert_least_privilege(&s, &me).await;
    }
}

/// 请求头核对过是归属人（核对见 `contacts_gate_tests`）：照旧按她的角色。
#[tokio::test]
async fn keeper_named_in_header_keeps_her_level() {
    let s = home().await;
    let mut me = private_bot();
    me.sender = Some(TurnSender::Local(ALICE.into()));
    me.ai_level = Some(s.bus.ai_cap(&me));
    assert_eq!(me.ai_level, Some(AiLevel::SelfConfirms));
    assert!(names(&s, &me).await.contains(&ARCHIVE.to_owned()));
}

/// 没带头、有进行中的轮次：按那几轮里级别最低的人算，不按只读。
#[tokio::test]
async fn live_turns_without_header_use_the_lowest_turn() {
    let s = home().await;
    let admin_approves = role(&s, "需管理员批删除", AiLevel::AdminApprovesDeletes, true);
    member(&s, "bob", &admin_approves);
    let ledger = attach_ledger(&s);
    let bot = BotRef {
        plugin_id: AGENT.into(),
        bot: "household".into(),
    };
    start_turn(&ledger, &bot, ALICE, "alice-turn");
    start_turn(&ledger, &bot, "@bob:wonton.localhost", "bob-turn");

    let me = resolved(&s, &household(), None);
    assert_eq!(me.ai_level, Some(AiLevel::AdminApprovesDeletes));
    assert!(names(&s, &me).await.contains(&ARCHIVE.to_owned()));

    // 没经过请求头解析、直接构造的调用方（`sender` 为空）也按进行中的轮次算。
    assert_eq!(s.bus.ai_cap(&household()), AiLevel::AdminApprovesDeletes);
}

/// `wonton__send_message` 不按说话人封顶（归属人自己说话才放宽时段）：缺说话人时每条都发卡，没有“本次都放行”。
#[tokio::test]
async fn send_message_without_sender_asks_the_keeper_every_time() {
    let s = home().await;
    attach_ledger(&s);
    let id = PluginId::parse(AGENT).unwrap();
    s.manager.bot_records_for_test(
        &id,
        &[(
            "alice-bot",
            "@alice-bot:wonton.localhost",
            BotKeeper::User {
                username: "alice".into(),
            },
            "token",
        )],
    );
    let read_only = role(&s, "只读", AiLevel::ReadOnly, false);
    member(&s, "mom", &read_only);
    let me = resolved(&s, &private_bot(), None);
    assert_eq!(me.sender, None);
    assert_eq!(me.ai_level, Some(AiLevel::ReadOnly));

    for (n, body) in ["第一条", "第二条"].into_iter().enumerate() {
        let call = s.spawn_as(&me, SEND, json!({"recipient": "mom", "body": body}));
        let card = s.cards.wait_for(n + 1).await;
        assert_eq!(
            card.category,
            Some(ConfirmationCategory::SendMessage {
                recipient: "mom".into(),
                body: body.into(),
            })
        );
        s.answer(&card.id, Answer::AllowWindow).unwrap();
        let result = call.await.unwrap();
        assert!(!failed(&result), "{result:?}");
        assert_eq!(
            confirmation_text(&result).as_deref(),
            Some("card"),
            "第 {} 条照样发卡，不走时段",
            n + 1
        );
    }
    assert_eq!(s.cards.posted.lock().unwrap().len(), 2, "两条各一张");
}

fn confirmation_text(result: &rmcp::model::CallToolResult) -> Option<String> {
    result
        .meta
        .as_ref()?
        .0
        .get("app.wonton/confirmation")
        .and_then(Value::as_str)
        .map(str::to_owned)
}
