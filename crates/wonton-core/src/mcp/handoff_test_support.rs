//! Shared bounded assertions and the ACP provider used by handoff integration tests.

use crate::config::PublicUrls;
use crate::gateway::{Gateway, agent_acp::Registry, agent_chains::AgentChains};
use crate::http::HttpClient;
use crate::plugin::{
    PluginId, PluginManager, PluginStatus, docker::Engine, languages::Languages,
    upstreams::Endpoint,
};
use std::sync::Arc;
use std::time::Duration;
use wonton_protocol::{BotKeeper, ChainLimits, ChainTurnReport};

use rmcp::model::CallToolResult;
use serde_json::Value;

pub(crate) use crate::plugin::tests::agent_acp_contract::minimal_provider::{
    ControlledProvider, serve_controlled,
};

/// Wait only for observable state, with the same bound as the ACP contract tests.
pub(crate) async fn wait_until(what: &str, mut done: impl FnMut() -> bool) {
    for _ in 0..40 {
        if done() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("timed out waiting for {what}");
}

pub(crate) fn result_json(result: &CallToolResult) -> Value {
    serde_json::from_str(&super::tests::text(result)).expect("tool must return JSON text")
}

pub(crate) fn success(result: &CallToolResult) -> Value {
    assert_ne!(
        result.is_error,
        Some(true),
        "unexpected tool failure: {result:?}"
    );
    result_json(result)
}

pub(super) const A: &str = "com.example.orchestrator";
pub(super) const B: &str = "com.example.runtime";
pub(super) const C: &str = "com.example.worker";

pub(super) struct Bed {
    pub bus: std::sync::Arc<super::Bus>,
    pub caller: super::Caller,
    pub chains: std::sync::Arc<crate::gateway::agent_chains::AgentChains>,
    pub manager: std::sync::Arc<crate::plugin::PluginManager>,
    pub cards: std::sync::Arc<super::test_support::Cards>,
    pub provider: ControlledProvider,
    pub worker: ControlledProvider,
    pub gateway: std::sync::Arc<crate::gateway::Gateway>,
    pub dir: tempfile::TempDir,
}

pub(super) fn bot(id: &str) -> wonton_protocol::BotRef {
    wonton_protocol::BotRef {
        plugin_id: id.into(),
        bot: "default".into(),
    }
}

pub(super) async fn bed(declares: bool, user: &str) -> Bed {
    bed_for_keeper(declares, user, "owner").await
}

pub(super) async fn bed_for_keeper(declares: bool, user: &str, keeper: &str) -> Bed {
    let dir = tempfile::tempdir().unwrap();
    let (fake, config) = crate::matrix::fake::start().await;
    for name in ["owner", "alice", "bot-a", "bot-b", "bot-c"] {
        fake.state()
            .users
            .insert(format!("@{name}:test"), Default::default());
    }
    let matrix = Arc::new(
        crate::matrix::Matrix::connect(&config, &dir.path().join("core"), HttpClient::new())
            .await
            .unwrap(),
    );
    let manager = super::test_setup::manager(Engine::unreachable_for_test(), &dir);
    seed_plugins(&manager, declares);
    manager.bot_records_for_test(
        &PluginId::parse(B).unwrap(),
        &[(
            "default",
            "@bot-b:test",
            BotKeeper::User {
                username: keeper.into(),
            },
            "token",
        )],
    );
    let token = manager
        .ensure_provider_token("agent_acp", &PluginId::parse(B).unwrap())
        .unwrap();
    let provider = serve_controlled(token).await;
    let worker_token = manager
        .ensure_provider_token("agent_acp", &PluginId::parse(C).unwrap())
        .unwrap();
    let worker = serve_controlled(worker_token).await;
    manager.capability_route_for_test(
        &PluginId::parse(A).unwrap(),
        "agent_acp",
        &PluginId::parse(B).unwrap(),
        Endpoint::Tcp(provider.addr.to_string()),
    );
    let cards = Arc::new(super::test_support::Cards::default());
    let bus = super::Bus::new(
        manager.clone(),
        Arc::new(super::Confirmations::new(
            cards.clone(),
            Duration::from_secs(5),
        )),
        Arc::new(super::audit::Audit::new(dir.path().join("audit.jsonl"))),
    );
    let chains = chain_service(&dir);
    let registry = Registry::new();
    chains.attach_acp_cancel(registry.clone());
    bus.attach_agent_chains(chains.clone());
    let (tunnel, members) = crate::gateway::fakes::loopback_tunnel(dir.path()).await;
    let roles = Arc::new(crate::roles::Roles::open(dir.path().join("roles.json")).unwrap());
    bus.attach_people("test".into(), members.clone(), roles);
    manager.note_person_languages("owner", Some("en"));
    let contacts = if keeper == "owner" {
        None
    } else {
        Some(target_contacts(&dir, manager.clone(), members.clone(), keeper).await)
    };
    let gateway = gateway(
        &dir,
        manager.clone(),
        &bus,
        chains.clone(),
        registry,
        matrix,
        members,
        tunnel,
        contacts,
    );
    bus.attach_handoff_gateway(Arc::downgrade(&gateway));
    let caller = caller(&chains, user);
    Bed {
        bus,
        caller,
        chains,
        manager,
        cards,
        provider,
        worker,
        gateway,
        dir,
    }
}

fn seed_plugins(manager: &PluginManager, declares: bool) {
    for (id, extra) in [
        (
            A,
            if declares {
                "  wants: [agent_acp]\n  provides: {agent_acp: {service: gateway, port: 8080}}"
            } else {
                ""
            },
        ),
        (
            B,
            "  wants: [agent_acp]\n  provides: {agent_acp: {service: gateway, port: 8080}}",
        ),
        (
            C,
            "  wants: [agent_acp]\n  provides: {agent_acp: {service: gateway, port: 8080}}",
        ),
    ] {
        let source = format!(
            "services:\n  gateway:\n    image: example/agent\nx-wonton:\n  spec: '0.11'\n  id: {id}\n  version: 1.0.0\n  title: {{en: {}}}\n  description: {{en: Agent}}\n  kind: app\n  permissions: []\n{extra}\n",
            id
        );
        let package =
            crate::plugin::PluginPackage::parse(&source, &Languages::from_header(Some("en")))
                .unwrap();
        manager.register_for_test(package.manifest, PluginStatus::Running, None);
        manager.bot_records_for_test(
            &PluginId::parse(id).unwrap(),
            &[(
                "default",
                match id {
                    A => "@bot-a:test",
                    B => "@bot-b:test",
                    C => "@bot-c:test",
                    _ => unreachable!("only registered test plugins"),
                },
                BotKeeper::User {
                    username: "owner".into(),
                },
                "token",
            )],
        );
    }
}

fn chain_service(dir: &tempfile::TempDir) -> Arc<AgentChains> {
    let ledger = Arc::new(
        crate::agent_chain::Chains::open(
            dir.path().join("agent-chains.json"),
            ChainLimits::default(),
        )
        .unwrap(),
    );
    let settings = Arc::new(
        crate::agent_chain::settings::AgentSettings::open(dir.path().join("agent-settings.json"))
            .unwrap(),
    );
    Arc::new(AgentChains::new(
        ledger,
        Arc::new(crate::agent_outbox::Outbox::default()),
        settings,
    ))
}

#[allow(clippy::too_many_arguments)]
fn gateway(
    dir: &tempfile::TempDir,
    manager: Arc<PluginManager>,
    bus: &Arc<super::Bus>,
    chains: Arc<AgentChains>,
    registry: Arc<Registry>,
    matrix: Arc<crate::matrix::Matrix>,
    members: Arc<crate::members::Members>,
    tunnel: crate::tunnel::Tunnel,
    contacts: Option<Arc<crate::contacts::Contacts>>,
) -> Arc<Gateway> {
    Arc::new(
        Gateway::new(
            PublicUrls {
                scheme: "http".into(),
                base_domain: "test".into(),
                port: 18480,
            },
            crate::owner::OwnerStore::for_test(
                dir.path(),
                &crate::owner::Owner {
                    username: "owner".into(),
                    display_name: "Owner".into(),
                    email: None,
                },
            ),
            "owner-token".into(),
            members,
            manager,
            None,
            HttpClient::new(),
            tunnel,
        )
        .with_matrix(Some(matrix))
        .with_mcp(Some(bus.endpoint()))
        .with_agent_chains(Some(chains))
        .with_agent_acp(Some(registry))
        .with_contacts(contacts),
    )
}

fn caller(chains: &AgentChains, user: &str) -> super::Caller {
    let grant = chains
        .ledger()
        .start_bridge_turn(
            bot(A),
            ChainTurnReport::Start {
                session: "caller-session".into(),
                room: "!conversation:test".into(),
                thread: Some("$thread:test".into()),
                sender: user.into(),
                event_id: "$trigger:test".into(),
            },
            None,
            user.into(),
        )
        .unwrap();
    let mut caller = super::Caller::for_bot(
        PluginId::parse(A).unwrap(),
        None,
        "default".into(),
        BotKeeper::User {
            username: "owner".into(),
        },
    );
    caller.name = "A".into();
    caller.chain = Some(grant.chain);
    caller.session = Some("caller-session".into());
    caller.request_id = Some(rmcp::model::NumberOrString::Number(1));
    caller
}

async fn target_contacts(
    dir: &tempfile::TempDir,
    manager: Arc<PluginManager>,
    members: Arc<crate::members::Members>,
    keeper: &str,
) -> Arc<crate::contacts::Contacts> {
    let invites = Arc::new(crate::contacts::tests::FakeInvites::default());
    let owner = crate::owner::OwnerStore::for_test(
        dir.path(),
        &crate::owner::Owner {
            username: "owner".into(),
            display_name: "Owner".into(),
            email: None,
        },
    );
    let directory = Arc::new(crate::contacts::wiring::PluginDirectory::new(
        manager, members, owner,
    ));
    let contacts = crate::contacts::Contacts::open(
        dir.path().join("contacts.json"),
        "test",
        crate::contacts::Ports {
            directory,
            link: Arc::new(NoPeers),
            invites: invites.clone(),
            effects: Arc::new(crate::contacts::tests::FakeEffects::default()),
            open_federation: false,
        },
    )
    .unwrap();
    contacts
        .request(
            &bot(B),
            keeper,
            &wonton_protocol::ContactRequest {
                user_id: "@owner:test".into(),
                kind: wonton_protocol::ContactKind::Person,
            },
        )
        .await
        .unwrap();
    invites.states.lock().unwrap().insert(
        "!dm-@owner:test".into(),
        crate::contacts::InviteeState::Joined,
    );
    contacts.check_people().await;
    contacts
}

struct NoPeers;

impl crate::contacts::PeerLink for NoPeers {
    fn reach(&self, _: &str) -> crate::contacts::PeerReach {
        crate::contacts::PeerReach::NotPaired
    }
    fn send(
        &self,
        _: &str,
        _: wonton_protocol::ContactEvent,
    ) -> futures_util::future::BoxFuture<'_, Result<(), crate::contacts::ContactFailure>> {
        Box::pin(async {
            Err(crate::contacts::ContactFailure::new(
                wonton_protocol::ContactError::Upstream,
                "not paired",
            ))
        })
    }
}

impl Bed {
    pub(super) fn route(&self, consumer: &str, provider: &str) {
        self.manager.capability_route_for_test(
            &PluginId::parse(consumer).unwrap(),
            "agent_acp",
            &PluginId::parse(provider).unwrap(),
            Endpoint::Tcp(
                match provider {
                    B => self.provider.addr,
                    C => self.worker.addr,
                    _ => unreachable!("only registered test providers"),
                }
                .to_string(),
            ),
        );
    }

    pub(super) fn child_caller(&self, id: &str, session: &str, request: u32) -> super::Caller {
        let mut caller = super::Caller::for_bot(
            PluginId::parse(id).unwrap(),
            None,
            "default".into(),
            BotKeeper::User {
                username: "owner".into(),
            },
        );
        caller.session = Some(session.to_owned());
        caller.chain.clone_from(&self.caller.chain);
        caller.request_id = Some(rmcp::model::NumberOrString::Number(request.into()));
        caller
    }

    pub(super) async fn finish(&self, session: &str, prompt: usize) {
        let target = self.chains.ledger().session_chain(session).unwrap().bot;
        let provider = if target.plugin_id == C {
            &self.worker
        } else {
            &self.provider
        };
        provider.recording.wait_prompt(prompt).await;
        provider.finish(session).await;
        wait_until("ended handoff", || {
            self.chains
                .ledger()
                .session_chain(session)
                .is_some_and(|info| !info.live)
        })
        .await;
    }

    pub(super) async fn approved(
        &self,
        caller: &super::Caller,
        target: &str,
        message: &str,
        card: usize,
    ) -> Value {
        let call = self.spawn(
            caller,
            serde_json::json!({"target_bot":bot(target),"message":message}),
        );
        let card = self.cards.wait_for(card).await;
        self.bus
            .confirmations
            .answer(&card.id, wonton_protocol::Answer::Approve, None)
            .unwrap();
        success(
            &tokio::time::timeout(Duration::from_secs(2), call)
                .await
                .expect("handoff stalled")
                .unwrap(),
        )
    }

    pub(super) async fn call(
        &self,
        caller: &super::Caller,
        tool: &str,
        args: Value,
    ) -> CallToolResult {
        let request = rmcp::model::CallToolRequestParams::new(tool.to_owned())
            .with_arguments(args.as_object().unwrap().clone());
        tokio::time::timeout(
            Duration::from_secs(2),
            self.bus.call(caller, request, std::future::pending()),
        )
        .await
        .expect("tool did not return before prompt completion")
    }

    pub(super) fn spawn(
        &self,
        caller: &super::Caller,
        args: Value,
    ) -> tokio::task::JoinHandle<CallToolResult> {
        let bus = self.bus.clone();
        let caller = caller.clone();
        tokio::spawn(async move {
            let request = rmcp::model::CallToolRequestParams::new("wonton__hand_off")
                .with_arguments(args.as_object().unwrap().clone());
            bus.call(&caller, request, std::future::pending()).await
        })
    }

    pub(super) async fn open(&self, message: &str) -> Value {
        let call = self.spawn(
            &self.caller,
            serde_json::json!({"target_bot":bot(B),"message":message}),
        );
        let card = self.cards.wait_for(1).await;
        self.bus
            .confirmations
            .answer(&card.id, wonton_protocol::Answer::Approve, None)
            .unwrap();
        success(
            &tokio::time::timeout(Duration::from_secs(2), call)
                .await
                .expect("handoff did not return")
                .unwrap(),
        )
    }

    pub(super) fn permission_http(
        &self,
        request: Value,
    ) -> tokio::task::JoinHandle<hyper::StatusCode> {
        use tower::ServiceExt;
        self.manager.register_source_for_test(
            B,
            "127.0.0.0/8",
            vec![crate::plugin::Permission::Tools],
        );
        let app = crate::gateway::router(self.gateway.clone());
        tokio::spawn(async move {
            let mut request = hyper::Request::builder()
                .method("POST")
                .uri("/agent/permissions/token")
                .header("host", "mcp.wonton.internal:18480")
                .header("content-type", "application/json")
                .body(axum::body::Body::from(request.to_string()))
                .unwrap();
            request.extensions_mut().insert(axum::extract::ConnectInfo(
                "127.0.0.1:12345".parse::<std::net::SocketAddr>().unwrap(),
            ));
            app.oneshot(request).await.unwrap().status()
        })
    }
}
