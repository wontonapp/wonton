use serde_json::{Value, json};
use wonton_protocol::{Answer, ConfirmationCategory, HandOffState};

use super::handoff_test_support::{A, B, Bed, bed, bot, success, wait_until};

fn kind(result: &rmcp::model::CallToolResult) -> Option<&str> {
    result.meta.as_ref()?.0.get("app.wonton/failure")?.as_str()
}

fn args(message: &str) -> Value {
    json!({"target_bot":bot(B),"message":message})
}

async fn status(bed: &Bed, session: &str) -> Value {
    success(
        &bed.call(
            &bed.caller,
            "wonton__hand_off_status",
            json!({"session":session}),
        )
        .await,
    )[0]
    .clone()
}

#[tokio::test]
async fn starts_one_prompt_on_the_callers_chain_without_waiting_for_completion() {
    let bed = bed(true, "@owner:test").await;
    let opened = bed.open("修登录页").await;
    let session = opened["session"].as_str().unwrap();
    assert_eq!(opened["chain"], bed.caller.chain.as_ref().unwrap().as_str());
    let prompt = bed.provider.recording.wait_prompt(0).await;
    assert_eq!(prompt["params"]["sessionId"], session);
    assert_eq!(
        prompt["params"]["prompt"],
        json!([{"type":"text","text":"修登录页"}])
    );
    assert_eq!(bed.provider.recording.method_count("session/new"), 1);
    let new = &bed.provider.recording.received("session/new")[0];
    let meta = new["params"]["_meta"].as_object().unwrap();
    assert_eq!(
        meta.keys()
            .map(String::as_str)
            .collect::<std::collections::BTreeSet<_>>(),
        ["app.wonton/task", "app.wonton/chain"]
            .into_iter()
            .collect()
    );
    assert_eq!(meta["app.wonton/task"]["room"], "!conversation:test");
    assert_eq!(meta["app.wonton/task"]["thread"], "$thread:test");
    assert_eq!(
        meta["app.wonton/chain"],
        json!({"id":bed.caller.chain,"depth":2})
    );
    let row = bed
        .chains
        .ledger()
        .row(bed.caller.chain.as_ref().unwrap())
        .unwrap();
    assert_eq!(row.rounds, 2);
    assert_eq!(
        row.bots
            .iter()
            .map(|bot| (bot.bot.clone(), bot.layer))
            .collect::<Vec<_>>(),
        [(bot(A), 1), (bot(B), 2)]
    );
    assert_eq!(status(&bed, session).await["status"], "running");
    bed.provider.finish(session).await;
}

#[tokio::test]
async fn retrying_the_request_id_reuses_the_open_session() {
    let bed = bed(true, "@owner:test").await;
    let opened = bed.open("修登录页").await;
    let repeated = success(
        &bed.call(&bed.caller, "wonton__hand_off", args("修登录页"))
            .await,
    );
    assert_eq!(repeated, opened);
    bed.provider.recording.wait_prompt(0).await;
    assert_eq!(bed.provider.recording.method_count("session/new"), 1);
    assert_eq!(bed.provider.recording.method_count("session/prompt"), 1);
    bed.provider
        .finish(opened["session"].as_str().unwrap())
        .await;
}

#[tokio::test]
async fn permission_wait_and_finished_tail_are_observable_without_calling_the_provider() {
    let bed = bed(true, "@owner:test").await;
    let opened = bed.open("修登录页").await;
    let session = opened["session"].as_str().unwrap().to_owned();
    bed.provider.recording.wait_prompt(0).await;
    let permission = bed.permission_http(json!({
        "session":session,"tool_call_id":"edit","title":"Edit login",
        "options":[{"choice_id":"yes","kind":"allow_once","name":"Allow"},{"choice_id":"no","kind":"reject_once","name":"Reject"}]
    }));
    let card = bed.cards.wait_for(2).await;
    let frames = bed.provider.recording.frame_count();
    assert_eq!(status(&bed, &session).await["status"], "waiting_permission");
    assert_eq!(bed.provider.recording.frame_count(), frames);
    bed.bus
        .confirmations
        .answer(&card.id, Answer::Approve, Some("yes"))
        .unwrap();
    assert_eq!(permission.await.unwrap(), hyper::StatusCode::OK);
    let text = format!("{}末尾🙂", "汉".repeat(2100));
    bed.provider.text(&session, &text).await;
    bed.provider.finish(&session).await;
    wait_until("ended handoff", || {
        bed.bus
            .handoffs
            .list(
                &bed.bus,
                &bed.caller,
                Some(&session),
                &wonton_protocol::ConfirmationOrigin {
                    room_id: "!conversation:test".into(),
                    thread_id: Some("$thread:test".into()),
                },
            )
            .unwrap()[0]
            .status
            == HandOffState::Ended
    })
    .await;
    let finished = status(&bed, &session).await;
    assert_eq!(finished["status"], "ended");
    assert_eq!(finished["stop_reason"], "end_turn");
    let expected = text
        .chars()
        .skip(text.chars().count() - 2000)
        .collect::<String>();
    assert_eq!(finished["summary"], expected);
    assert_eq!(finished["summary"].as_str().unwrap().chars().count(), 2000);
}

#[tokio::test]
async fn stopping_a_chain_cancels_the_session_and_rejects_new_handoffs() {
    let bed = bed(true, "@owner:test").await;
    let opened = bed.open("修登录页").await;
    let session = opened["session"].as_str().unwrap();
    bed.provider.recording.wait_prompt(0).await;
    bed.bus
        .stop_chain(bed.caller.chain.as_ref().unwrap())
        .await
        .unwrap();
    wait_until("session cancellation", || {
        bed.provider.recording.method_count("session/cancel") == 1
    })
    .await;
    assert_eq!(status(&bed, session).await["status"], "stopped");
    let refused = bed
        .call(&bed.caller, "wonton__hand_off", args("另一件事"))
        .await;
    assert_eq!(kind(&refused), Some("stopped"));
    assert_eq!(bed.provider.recording.method_count("session/new"), 1);
}

#[tokio::test]
async fn only_the_dispatching_bot_can_query_and_queries_do_not_count_rounds() {
    let bed = bed(true, "@owner:test").await;
    let opened = bed.open("修登录页").await;
    let session = opened["session"].as_str().unwrap();
    let chain = bed.caller.chain.as_ref().unwrap();
    let rounds = bed.chains.ledger().row(chain).unwrap().rounds;
    assert_eq!(
        success(
            &bed.call(&bed.caller, "wonton__hand_off_status", json!({}))
                .await
        )
        .as_array()
        .unwrap()
        .len(),
        1
    );
    let mut foreign = bed.caller.clone();
    foreign.id = crate::plugin::PluginId::parse(B).unwrap();
    foreign.session = Some(session.into());
    let refused = bed
        .call(
            &foreign,
            "wonton__hand_off_status",
            json!({"session":session}),
        )
        .await;
    assert_eq!(kind(&refused), Some("permission_denied"));
    assert_eq!(bed.chains.ledger().row(chain).unwrap().rounds, rounds);
    bed.provider.recording.wait_prompt(0).await;
    bed.provider.finish(session).await;
}

#[tokio::test]
async fn a_family_chain_cannot_connect_to_the_owners_private_bot() {
    let bed = bed(true, "@alice:test").await;
    let refused = bed
        .call(&bed.caller, "wonton__hand_off", args("修登录页"))
        .await;
    assert_eq!(kind(&refused), Some("permission_denied"));
    assert_eq!(bed.provider.recording.connections(), 0);
    assert!(bed.cards.posted.lock().unwrap().is_empty());
}

#[tokio::test]
async fn first_connection_asks_the_target_keeper_and_rejection_never_connects() {
    let bed = bed(true, "@owner:test").await;
    let call = bed.spawn(&bed.caller, args("修登录页"));
    let card = bed.cards.wait_for(1).await;
    assert_eq!(
        card.category,
        Some(ConfirmationCategory::HandOffConsent {
            target_bot: bot(B),
            from: bot(A)
        })
    );
    assert_eq!(bed.bus.confirmations.recipient(&card.id), Some(None));
    assert_eq!(bed.provider.recording.connections(), 0);
    bed.bus
        .confirmations
        .answer(&card.id, Answer::Reject, None)
        .unwrap();
    let refused = call.await.unwrap();
    assert_eq!(kind(&refused), Some("owner_rejected"));
    assert_eq!(bed.provider.recording.connections(), 0);
}

#[tokio::test]
async fn a_plugin_without_the_capability_is_denied() {
    let bed = bed(false, "@owner:test").await;
    let refused = bed
        .call(&bed.caller, "wonton__hand_off", args("修登录页"))
        .await;
    assert_eq!(kind(&refused), Some("permission_denied"));
    let refused = bed
        .call(&bed.caller, "wonton__hand_off_status", json!({}))
        .await;
    assert_eq!(kind(&refused), Some("permission_denied"));
    assert_eq!(bed.provider.recording.connections(), 0);
}

#[tokio::test]
async fn someone_elses_bot_asks_its_keeper_before_connecting() {
    let bed = super::handoff_test_support::bed_for_keeper(true, "@owner:test", "alice").await;
    let call = bed.spawn(&bed.caller, args("修登录页"));
    let card = bed.cards.wait_for(1).await;
    assert_eq!(
        card.category,
        Some(ConfirmationCategory::HandOffConsent {
            target_bot: bot(B),
            from: bot(A)
        })
    );
    assert_eq!(
        bed.bus.confirmations.recipient(&card.id),
        Some(Some("alice".into()))
    );
    assert_eq!(bed.provider.recording.connections(), 0);
    bed.bus
        .confirmations
        .answer(&card.id, Answer::Reject, None)
        .unwrap();
    assert_eq!(kind(&call.await.unwrap()), Some("owner_rejected"));
    assert_eq!(bed.provider.recording.connections(), 0);
}

#[tokio::test]
async fn handler_resolves_the_authenticated_session_and_json_rpc_request_id() {
    use rmcp::model::{CallToolRequestParams, ClientCapabilities, ClientConfig, Implementation};
    use rmcp::transport::{
        StreamableHttpClientTransport, streamable_http_client::StreamableHttpClientTransportConfig,
    };
    use rmcp::{ClientLifecycleMode, ClientServiceExt};
    let bed = bed(true, "@owner:test").await;
    bed.manager
        .register_source_for_test(A, "127.0.0.0/8", vec![crate::plugin::Permission::Tools]);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = crate::gateway::router(bed.gateway.clone());
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .await
        .unwrap();
    });
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert(
        wonton_protocol::SESSION_HEADER,
        reqwest::header::HeaderValue::from_static("caller-session"),
    );
    let http = reqwest::Client::builder()
        .no_proxy()
        .resolve("mcp.wonton.internal", addr)
        .default_headers(headers)
        .build()
        .unwrap();
    let config = StreamableHttpClientTransportConfig::with_uri(format!(
        "http://mcp.wonton.internal:{}/mcp/token",
        addr.port()
    ));
    let transport = StreamableHttpClientTransport::with_client(http, config);
    let mut agent = ClientConfig::new(
        ClientCapabilities::default(),
        Implementation::new("handoff-test", "0"),
    )
    .serve_with_lifecycle(transport, ClientLifecycleMode::Initialize)
    .await
    .unwrap();
    let tools = agent.peer().list_all_tools().await.unwrap();
    assert!(tools.iter().any(|tool| tool.name == "wonton__hand_off"));
    let peer = agent.peer().clone();
    let call = tokio::spawn(async move {
        peer.call_tool(
            CallToolRequestParams::new("wonton__hand_off")
                .with_arguments(args("修登录页").as_object().unwrap().clone()),
        )
        .await
        .unwrap()
    });
    let card = bed.cards.wait_for(1).await;
    bed.bus
        .confirmations
        .answer(&card.id, Answer::Approve, None)
        .unwrap();
    let result = tokio::time::timeout(std::time::Duration::from_secs(2), call)
        .await
        .expect("handler waited for B to finish")
        .unwrap();
    let opened = success(&result);
    assert_eq!(opened["chain"], bed.caller.chain.as_ref().unwrap().as_str());
    assert_eq!(bed.provider.recording.method_count("session/new"), 1);
    bed.provider.recording.wait_prompt(0).await;
    let statuses = agent
        .peer()
        .call_tool(
            CallToolRequestParams::new("wonton__hand_off_status")
                .with_arguments(json!({}).as_object().unwrap().clone()),
        )
        .await
        .unwrap();
    assert_eq!(success(&statuses)[0]["session"], opened["session"]);
    bed.provider
        .finish(opened["session"].as_str().unwrap())
        .await;
    agent.close().await.unwrap();
    server.abort();
}

#[tokio::test]
async fn invalid_or_foreign_caller_sessions_never_open_a_provider_connection() {
    let bed = bed(true, "@owner:test").await;
    let mut missing = bed.caller.clone();
    missing.session = None;
    let refused = bed
        .call(&missing, "wonton__hand_off", args("修登录页"))
        .await;
    assert_eq!(kind(&refused), Some("permission_denied"));
    let mut foreign = bed.caller.clone();
    foreign.id = crate::plugin::PluginId::parse(B).unwrap();
    let refused = bed
        .call(&foreign, "wonton__hand_off", args("修登录页"))
        .await;
    assert_eq!(kind(&refused), Some("permission_denied"));
    let refused = bed.call(&bed.caller, "wonton__hand_off", args("  ")).await;
    assert_eq!(kind(&refused), Some("invalid_input"));
    assert_eq!(bed.provider.recording.connections(), 0);
    assert!(bed.cards.posted.lock().unwrap().is_empty());
}

#[tokio::test]
async fn another_caller_session_is_not_deduplicated_and_lists_only_its_conversation() {
    let bed = bed(true, "@owner:test").await;
    let first = bed.open("修登录页").await;
    let mut caller = bed.caller.clone();
    let grant = bed
        .chains
        .ledger()
        .start_bridge_turn(
            bot(A),
            wonton_protocol::ChainTurnReport::Start {
                session: "another-caller".into(),
                room: "!conversation:test".into(),
                thread: Some("$other-thread:test".into()),
                sender: "@owner:test".into(),
                event_id: "$other:test".into(),
            },
            None,
            "@owner:test".into(),
        )
        .unwrap();
    caller.session = Some("another-caller".into());
    caller.chain = Some(grant.chain);
    let second = success(
        &bed.call(&caller, "wonton__hand_off", args("修登录页"))
            .await,
    );
    assert_ne!(first["session"], second["session"]);
    assert_ne!(first["chain"], second["chain"]);
    assert_eq!(
        bed.cards.posted.lock().unwrap().len(),
        1,
        "自己的已确认Bot不再问"
    );
    let listed = success(
        &bed.call(&caller, "wonton__hand_off_status", json!({}))
            .await,
    );
    assert_eq!(listed.as_array().unwrap().len(), 1);
    assert_eq!(listed[0]["session"], second["session"]);
    let listed = success(
        &bed.call(&bed.caller, "wonton__hand_off_status", json!({}))
            .await,
    );
    assert_eq!(listed.as_array().unwrap().len(), 1);
    assert_eq!(listed[0]["session"], first["session"]);
    bed.provider
        .finish(first["session"].as_str().unwrap())
        .await;
    bed.provider.recording.wait_prompt(1).await;
    bed.provider
        .finish(second["session"].as_str().unwrap())
        .await;
}

#[tokio::test]
async fn a_foreign_initiator_cannot_relax_target_tools_by_omitting_the_session_header() {
    let bed = super::handoff_test_support::bed_for_keeper(true, "@owner:test", "alice").await;
    let people = bed.bus.people.get().unwrap();
    let role = people
        .roles
        .create(&wonton_protocol::RoleDraft {
            name: "笔记使用者".into(),
            permissions: wonton_protocol::Permissions {
                open_apps: wonton_protocol::AppScope::Selected {
                    plugin_ids: vec!["app.example.notes".into()],
                },
                ai: wonton_protocol::AiLevel::SelfConfirms,
                ..wonton_protocol::Permissions::strictest()
            },
        })
        .unwrap();
    people
        .members
        .invite(crate::members::MemberRecord {
            role_id: role.id,
            ..crate::members::MemberRecord::for_test("alice")
        })
        .unwrap();
    let opened = bed.open("修登录页").await;
    let notes = target_notes(&bed).await;
    bed.manager.tool_policy().set_allow_all(B, true).unwrap();
    let target = super::Caller::for_bot(
        crate::plugin::PluginId::parse(B).unwrap(),
        None,
        "default".into(),
        wonton_protocol::BotKeeper::User {
            username: "alice".into(),
        },
    );
    let bus = bed.bus.clone();
    let called = tokio::spawn(async move {
        let request = rmcp::model::CallToolRequestParams::new("app-example-notes__delete")
            .with_arguments(json!({"name":"draft"}).as_object().unwrap().clone());
        bus.call(&target, request, std::future::pending()).await
    });
    let card = bed.cards.wait_for(2).await;
    assert_eq!(
        bed.bus.confirmations.recipient(&card.id),
        Some(Some("alice".into()))
    );
    assert!(
        card.choices
            .iter()
            .all(|choice| choice.answer != Answer::AllowWindow)
    );
    assert!(notes.deleted.lock().unwrap().is_empty());
    bed.bus
        .confirmations
        .answer(&card.id, Answer::Reject, None)
        .unwrap();
    assert_eq!(kind(&called.await.unwrap()), Some("owner_rejected"));
    assert!(notes.deleted.lock().unwrap().is_empty());
    bed.provider.recording.wait_prompt(0).await;
    bed.provider
        .finish(opened["session"].as_str().unwrap())
        .await;
}

#[tokio::test]
async fn a_targets_own_initiator_keeps_the_existing_tool_relaxations() {
    let bed = bed(true, "@owner:test").await;
    let opened = bed.open("修登录页").await;
    let notes = target_notes(&bed).await;
    bed.manager.tool_policy().set_allow_all(B, true).unwrap();
    let target = super::Caller::for_bot(
        crate::plugin::PluginId::parse(B).unwrap(),
        None,
        "default".into(),
        wonton_protocol::BotKeeper::User {
            username: "owner".into(),
        },
    );
    let result = bed
        .call(
            &target,
            "app-example-notes__delete",
            json!({"name":"draft"}),
        )
        .await;
    assert_ne!(result.is_error, Some(true), "{result:?}");
    assert_eq!(*notes.deleted.lock().unwrap(), ["draft"]);
    assert_eq!(bed.cards.posted.lock().unwrap().len(), 1);
    bed.provider.recording.wait_prompt(0).await;
    bed.provider
        .finish(opened["session"].as_str().unwrap())
        .await;
}

async fn target_notes(bed: &Bed) -> super::test_support::Notes {
    let package = crate::plugin::PluginPackage::parse(
        super::test_setup::NOTES,
        &bed.manager.owner_languages(),
    )
    .unwrap();
    let id = package.manifest.id.clone();
    bed.manager
        .register_for_test(package.manifest, crate::plugin::PluginStatus::Running, None);
    let notes = super::test_support::Notes::default();
    let url = super::test_support::serve_notes(notes.clone()).await;
    bed.manager.mcp_for_test(&id, url);
    notes
}

#[tokio::test]
async fn handoff_consent_survives_restart_without_changing_an_install_plan() {
    let bed = bed(true, "@owner:test").await;
    let package = crate::plugin::PluginPackage::parse(
        super::test_setup::NOTES,
        &bed.manager.owner_languages(),
    )
    .unwrap();
    let plan = bed.manager.dependency_plan(&package).unwrap();
    let opened = bed.open("修登录页").await;
    assert_eq!(bed.manager.dependency_plan(&package).unwrap(), plan);
    let restored = super::test_setup::manager(
        crate::plugin::docker::Engine::unreachable_for_test(),
        &bed.dir,
    );
    assert!(restored.handoff_confirmed(&bot(A), &bot(B)).unwrap());
    assert!(!restored.handoff_confirmed(&bot(B), &bot(A)).unwrap());
    let mut other = bot(A);
    other.bot = "other".into();
    assert!(!restored.handoff_confirmed(&other, &bot(B)).unwrap());
    bed.provider.recording.wait_prompt(0).await;
    bed.provider
        .finish(opened["session"].as_str().unwrap())
        .await;
}

#[tokio::test]
async fn another_bot_in_the_same_plugin_needs_its_own_handoff_card() {
    let bed = bed(true, "@owner:test").await;
    let first = bed.open("first task").await;
    bed.finish(first["session"].as_str().unwrap(), 0).await;
    let mut repeated = bed.caller.clone();
    repeated.request_id = Some(rmcp::model::NumberOrString::Number(2));
    let opened = success(
        &tokio::time::timeout(
            std::time::Duration::from_secs(2),
            bed.call(&repeated, "wonton__hand_off", args("second task")),
        )
        .await
        .unwrap(),
    );
    assert_eq!(bed.cards.posted.lock().unwrap().len(), 1);
    bed.finish(opened["session"].as_str().unwrap(), 1).await;
    let keeper = wonton_protocol::BotKeeper::User {
        username: "owner".into(),
    };
    bed.manager.bot_records_for_test(
        &crate::plugin::PluginId::parse(A).unwrap(),
        &[
            ("default", "@bot-a:test", keeper.clone(), "token"),
            ("other", "@bot-a-other:test", keeper.clone(), "other-token"),
        ],
    );
    let other = wonton_protocol::BotRef {
        plugin_id: A.into(),
        bot: "other".into(),
    };
    let grant = bed
        .chains
        .ledger()
        .start_bridge_turn(
            other.clone(),
            wonton_protocol::ChainTurnReport::Start {
                session: "other-bot-session".into(),
                room: "!conversation:test".into(),
                thread: Some("$thread:test".into()),
                sender: "@owner:test".into(),
                event_id: "$other:test".into(),
            },
            None,
            "@owner:test".into(),
        )
        .unwrap();
    let mut caller = super::Caller::for_bot(
        crate::plugin::PluginId::parse(A).unwrap(),
        None,
        "other".into(),
        keeper,
    );
    caller.session = Some("other-bot-session".into());
    caller.chain = Some(grant.chain);
    caller.request_id = Some(rmcp::model::NumberOrString::Number(1));
    let pending = bed.spawn(&caller, args("third task"));
    let card = bed.cards.wait_for(2).await;
    assert!(
        matches!(&card.category, Some(ConfirmationCategory::HandOffConsent { from, target_bot })
        if from == &other && target_bot == &bot(B))
    );
    bed.bus
        .confirmations
        .answer(&card.id, Answer::Approve, None)
        .unwrap();
    let opened = success(
        &tokio::time::timeout(std::time::Duration::from_secs(2), pending)
            .await
            .unwrap()
            .unwrap(),
    );
    assert_eq!(bed.cards.posted.lock().unwrap().len(), 2);
    bed.finish(opened["session"].as_str().unwrap(), 2).await;
}
