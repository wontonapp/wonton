//! 按 Bot 的模型接入路由（ADR-0129）：谁能动哪个 Bot、输入先在核心查、编码过的 ID 原样到插件、错误按类别给。

use hyper::{Method, StatusCode};
use iroh::{EndpointId, SecretKey};
use serde_json::{Value, json};
use wonton_protocol::{
    BotKeeper, PERMISSION_REQUIRED, agent_model_current_path, agent_model_list_path,
    agent_model_login_finish_path, agent_model_login_start_path, agent_model_provider_path,
    agent_models_path,
};

use super::super::family_tests::{bearer, send_as};
use super::super::tests::{CORE, Harness, Reply};
use crate::members::MemberRecord;
use crate::plugin::agent_models::support::{self, Agent, MODELS, PLUGIN};

async fn home(agent_extra: &str) -> (Harness, Agent) {
    let h = Harness::start(None).await;
    let agent = support::install(
        &h.manager,
        &h.docker,
        agent_extra,
        &[
            (
                "default",
                BotKeeper::User {
                    username: "owner".into(),
                },
            ),
            (
                "alice",
                BotKeeper::User {
                    username: "alice".into(),
                },
            ),
            ("shared", BotKeeper::Household),
        ],
    );
    (h, agent)
}

fn member(h: &Harness, name: &str, role: &str) -> EndpointId {
    h.members
        .invite(MemberRecord {
            role_id: role.into(),
            ..MemberRecord::for_test(name)
        })
        .unwrap();
    h.members.mark_joined(name).unwrap();
    let device = SecretKey::generate().public();
    h.tunnel
        .devices
        .add_if(device, None, Some(name.into()), |_| true)
        .unwrap();
    device
}

async fn owner(h: &Harness, method: Method, path: &str, body: Value) -> Reply {
    let body = if body.is_null() {
        String::new()
    } else {
        body.to_string()
    };
    send_as(
        &h.app,
        None,
        method,
        CORE,
        path,
        &[("authorization", &bearer())],
        &body,
    )
    .await
}

async fn device(h: &Harness, device: EndpointId, path: &str) -> Reply {
    send_as(&h.app, Some(device), Method::GET, CORE, path, &[], "").await
}

const CATALOG: &str = r#"{"providers": [], "current": null}"#;

#[tokio::test]
async fn the_owner_reads_any_bots_catalog() {
    let (h, agent) = home(MODELS).await;
    agent.reply(0, CATALOG, "");
    let reply = owner(
        &h,
        Method::GET,
        &agent_models_path(PLUGIN, "alice"),
        Value::Null,
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.text);
    assert_eq!(reply.json(), json!({"providers": [], "current": null}));
    assert_eq!(agent.seen()[0].input["bot"], "alice");
}

#[tokio::test]
async fn a_member_changes_only_her_own_bot_and_shared_ones_she_manages() {
    let (h, agent) = home(MODELS).await;
    agent.reply(0, CATALOG, "");
    let alice = member(&h, "alice", "admin");
    let carol = member(&h, "carol", "friend");
    let reply = device(&h, carol, &agent_models_path(PLUGIN, "alice")).await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN);
    assert_eq!(reply.error_kind(), Some(PERMISSION_REQUIRED));

    let own = device(&h, alice, &agent_models_path(PLUGIN, "alice")).await;
    assert_eq!(own.status, StatusCode::OK, "{}", own.text);
    let shared = device(&h, alice, &agent_models_path(PLUGIN, "shared")).await;
    assert_eq!(shared.status, StatusCode::OK, "{}", shared.text);
    let owners = device(&h, alice, &agent_models_path(PLUGIN, "default")).await;
    assert_eq!(owners.status, StatusCode::FORBIDDEN);
    assert_eq!(owners.error_kind(), Some("forbidden"));
    assert_eq!(owners.json()["error"]["kind"], "forbidden");
    assert_eq!(
        agent.seen().len(),
        2,
        "refused requests never reach the plugin"
    );
}

#[tokio::test]
async fn a_pasted_key_reaches_the_plugin_trimmed() {
    let (h, agent) = home(MODELS).await;
    agent.reply(0, r#"{"check": "accepted"}"#, "");
    let path = agent_model_provider_path(PLUGIN, "default", "openrouter");
    let reply = owner(&h, Method::PUT, &path, json!({"key": "  sk-or-1\n"})).await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.text);
    assert_eq!(reply.json()["check"], "accepted");
    let input = &agent.seen()[0].input;
    assert_eq!(
        (
            input["op"].as_str(),
            input["provider"].as_str(),
            input["key"].as_str()
        ),
        (Some("save_key"), Some("openrouter"), Some("sk-or-1"))
    );

    agent.reply(0, "", "");
    let gone = owner(&h, Method::DELETE, &path, Value::Null).await;
    assert_eq!(gone.status, StatusCode::NO_CONTENT);
    assert_eq!(agent.seen()[1].input["op"], "disconnect");
}

#[tokio::test]
async fn encoded_provider_ids_arrive_whole() {
    let (h, agent) = home(MODELS).await;
    agent.reply(
        0,
        r#"{"models": [{"id": "llama3", "recommended": true}]}"#,
        "",
    );
    let path = agent_model_list_path(PLUGIN, "default", "custom:local/1");
    let reply = owner(&h, Method::GET, &path, Value::Null).await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.text);
    assert_eq!(agent.seen()[0].input["provider"], "custom:local/1");
}

#[tokio::test]
async fn bad_input_is_refused_before_the_plugin_runs() {
    let (h, agent) = home(MODELS).await;
    let cases = [
        (
            Method::PUT,
            agent_model_provider_path(PLUGIN, "default", "openrouter"),
            json!({"key": "sk or"}),
            "key",
        ),
        (
            Method::PUT,
            agent_model_provider_path(PLUGIN, "default", "custom"),
            json!({"key": "sk-1", "base_url": "ftp://192.0.2.10/v1"}),
            "base_url",
        ),
        (
            Method::POST,
            agent_model_login_start_path(PLUGIN, "default", "openrouter"),
            json!({"redirect_uri": "https://example.com/callback"}),
            "redirect_uri",
        ),
        (
            Method::POST,
            agent_model_login_finish_path(PLUGIN, "default", "s1"),
            json!({"callback_url": "wonton://model-login-evil?code=1"}),
            "callback_url",
        ),
        (
            Method::PUT,
            agent_model_current_path(PLUGIN, "default"),
            json!({"provider": "nous", "model": " ", "confirmed": false}),
            "model",
        ),
    ];
    for (method, path, body, field) in cases {
        let reply = owner(&h, method, &path, body).await;
        assert_eq!(
            reply.status,
            StatusCode::BAD_REQUEST,
            "{path}: {}",
            reply.text
        );
        assert_eq!(reply.json()["error"]["kind"], "invalid_input");
        assert_eq!(reply.json()["error"]["field"], field, "{path}");
    }
    let garbled = owner(
        &h,
        Method::PUT,
        &agent_model_current_path(PLUGIN, "default"),
        json!("nope"),
    )
    .await;
    assert_eq!(garbled.json()["error"]["kind"], "invalid_input");
    assert!(agent.seen().is_empty());
}

#[tokio::test]
async fn failures_come_back_as_their_kind() {
    let (h, agent) = home("").await;
    let unsupported = owner(
        &h,
        Method::GET,
        &agent_models_path(PLUGIN, "default"),
        Value::Null,
    )
    .await;
    assert_eq!(unsupported.status, StatusCode::NOT_FOUND);
    assert_eq!(unsupported.json()["error"]["kind"], "unsupported");
    let (h, agent2) = home(MODELS).await;
    let missing = owner(
        &h,
        Method::GET,
        &agent_models_path(PLUGIN, "nobody"),
        Value::Null,
    )
    .await;
    assert_eq!(missing.json()["error"]["kind"], "not_installed");

    agent2.reply(7, "", "upstream 500");
    let upstream = owner(
        &h,
        Method::POST,
        &wonton_protocol::agent_model_test_path(PLUGIN, "default"),
        Value::Null,
    )
    .await;
    assert_eq!(upstream.status, StatusCode::BAD_GATEWAY);
    assert_eq!(upstream.json()["error"]["kind"], "upstream");
    assert!(
        upstream.json()["error"]["detail"]
            .as_str()
            .unwrap()
            .contains("upstream 500")
    );
    assert!(agent.seen().is_empty());
}
