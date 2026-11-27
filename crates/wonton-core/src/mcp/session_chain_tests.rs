//! 工具调用带回来的 `wonton-session` 认不认到链上：认到才挂链和会话；停了的链照样认（卡片绑上那刻再撤回）。

use std::sync::Arc;

use hyper::HeaderMap;
use wonton_protocol::{BotKeeper, BotRef, ChainTurnReport};

use super::Caller;
use super::test_setup::setup;
use crate::gateway::agent_chains::AgentChains;
use crate::plugin::PluginId;
use crate::plugin::fake_docker::FakeDocker;

const ID: &str = "com.example.agent";

async fn session_setup() -> (super::test_setup::Setup, Arc<AgentChains>, Caller) {
    let (_docker, engine) = FakeDocker::start().await;
    let s = setup(engine, std::time::Duration::from_secs(5)).await;
    let chains = AgentChains::for_test(s.dir.path()).unwrap();
    s.bus.attach_agent_chains(chains.clone());
    let caller = Caller::for_bot(
        PluginId::parse(ID).unwrap(),
        None,
        "main".into(),
        BotKeeper::User {
            username: "owner".into(),
        },
    );
    (s, chains, caller)
}

fn bot() -> BotRef {
    BotRef {
        plugin_id: ID.into(),
        bot: "main".into(),
    }
}

fn headers(session: &str) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(wonton_protocol::SESSION_HEADER, session.parse().unwrap());
    headers
}

/// 这条会话报过一次 Start，返回它的链。
fn start(chains: &AgentChains, session: &str) -> String {
    let grant = chains
        .ledger()
        .start_turn(
            bot(),
            ChainTurnReport::Start {
                session: session.into(),
                room: "!room:wonton.localhost".into(),
                thread: None,
                sender: "owner".into(),
                event_id: format!("${session}"),
            },
            None,
            "owner".into(),
        )
        .unwrap();
    grant.chain
}

#[tokio::test]
async fn a_live_session_keeps_its_chain() {
    let (s, chains, caller) = session_setup().await;
    let chain = start(&chains, "s-1");
    let found = super::server::chain_session(&s.bus, &caller, Some(&headers("s-1")));
    assert_eq!(found, Some((chain, "s-1".to_owned())));
}

#[tokio::test]
async fn a_stopped_session_still_names_its_chain() {
    // 停链只撤得走当时挂着的卡片；之后还挂上去的卡片，靠这条链在绑上那刻认出已停再撤回，
    // 所以停了的链照样交出来（审计记 stopped_session）。
    let (s, chains, caller) = session_setup().await;
    let chain = start(&chains, "s-1");
    chains.ledger().stop(&chain).unwrap();
    assert_eq!(
        super::server::chain_session(&s.bus, &caller, Some(&headers("s-1"))),
        Some((chain, "s-1".to_owned()))
    );
}

#[tokio::test]
async fn someone_elses_session_is_not_adopted() {
    let (s, chains, mut caller) = session_setup().await;
    start(&chains, "s-1");
    caller.bot = Some(super::server::BotIdentity {
        bot: "other".into(),
        keeper: BotKeeper::Household,
    });
    assert_eq!(
        super::server::chain_session(&s.bus, &caller, Some(&headers("s-1"))),
        None
    );
}

#[tokio::test]
async fn an_unknown_session_falls_back_to_no_chain() {
    let (s, _chains, caller) = session_setup().await;
    assert_eq!(
        super::server::chain_session(&s.bus, &caller, Some(&headers("nope"))),
        None
    );
}
