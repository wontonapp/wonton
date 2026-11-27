//! Agent 插件的模型接入（ADR-0129）：核心按 Bot 执行声明的命令，查过输出才转出，key 不回显。

use serde_json::json;
use wonton_protocol::{
    AgentModelsFailure, BotKeeper, KeyCheck, KeyCheckResult, LoginState, MAX_MODELS_OUTPUT_BYTES,
    ModelCatalog, ModelLogin, ModelLoginState, ModelSelection, ModelTest, ModelTestResult,
    ModelsOp, SecretString,
};

use super::super::PluginStatus;
use super::super::agent_models::support::{self, Agent, MODELS, PLUGIN};
use super::super::agent_models::{AgentModelsError, ModelsOutput};
use super::super::fake_docker::FakeDocker;
use super::super::languages::Languages;
use super::super::manager::PluginManager;
use super::super::package::PluginPackage;
use super::manager::manager_in_container;

struct Bed {
    manager: PluginManager,
    _docker: FakeDocker,
    agent: Agent,
    _dir: tempfile::TempDir,
}

fn langs() -> Languages {
    Languages::from_header(Some("zh-Hans, en;q=0.8"))
}

async fn bed(agent_extra: &str) -> Bed {
    let dir = tempfile::tempdir().unwrap();
    let (docker, engine) = FakeDocker::start().await;
    let manager = manager_in_container(engine, &dir);
    let agent = support::install(
        &manager,
        &docker,
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
        ],
    );
    Bed {
        manager,
        _docker: docker,
        agent,
        _dir: dir,
    }
}

impl Bed {
    async fn run<T: ModelsOutput>(&self, bot: &str, op: ModelsOp) -> Result<T, AgentModelsError> {
        let target = self.manager.agent_models_target(PLUGIN, bot, &langs())?;
        self.manager
            .run_agent_models::<T>(&target, op, &langs())
            .await
    }

    async fn fail(&self, op: ModelsOp) -> AgentModelsError {
        self.run::<ModelCatalog>("alice", op).await.unwrap_err()
    }
}

fn catalog(key_url: &str) -> String {
    json!({
        "providers": [{
            "id": "openrouter", "name": "OpenRouter", "recommended": true,
            "auth": [{"type": "api_key", "key_url": key_url, "key_prefix": "sk-or-"}],
            "connected": false
        }],
        "current": null
    })
    .to_string()
}

#[tokio::test]
async fn the_declared_command_runs_in_the_bots_own_container_with_one_json_line() {
    let bed = bed(MODELS).await;
    bed.agent.reply(0, &catalog("https://example.com/keys"), "");
    let catalog: ModelCatalog = bed.run("alice", ModelsOp::Catalog).await.unwrap();
    assert_eq!(catalog.providers[0].id, "openrouter");
    let seen = bed.agent.seen();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].container, support::container(&bed.manager, "alice"));
    assert_eq!(seen[0].argv, ["wonton-models", "--json"]);
    assert_eq!(
        seen[0].input,
        json!({"op": "catalog", "bot": "alice", "languages": ["zh-hans", "en"]})
    );
}

#[tokio::test]
async fn the_key_goes_in_on_stdin_and_never_comes_back_out() {
    let bed = bed(MODELS).await;
    let secret = "fake-test-key-not-a-secret";
    let save = || ModelsOp::SaveKey {
        provider: "openrouter".into(),
        key: SecretString::new(secret),
        base_url: None,
    };
    bed.agent.reply(
        0,
        &json!({"check": "rejected", "message": format!("{secret} 不对")}).to_string(),
        "",
    );
    let check: KeyCheck = bed.run("alice", save()).await.unwrap();
    assert_eq!(check.check, KeyCheckResult::Rejected);
    assert!(!check.message.unwrap().contains(secret));
    assert_eq!(bed.agent.seen()[0].input["key"], secret);

    bed.agent.reply(1, "", &format!("Traceback: key={secret}"));
    let error = bed.fail(save()).await;
    assert_eq!(error.kind, AgentModelsFailure::Upstream);
    let detail = error.detail.unwrap();
    assert!(
        detail.contains("key=[key]") && !detail.contains(secret),
        "{detail}"
    );
}

#[tokio::test]
async fn exit_codes_tell_the_app_what_went_wrong() {
    let bed = bed(MODELS).await;
    bed.agent
        .reply(2, r#"{"field": "base_url", "message": "地址打不开"}"#, "");
    let invalid = bed.fail(ModelsOp::Catalog).await;
    assert_eq!(
        (
            invalid.kind,
            invalid.field.as_deref(),
            invalid.message.as_str()
        ),
        (
            AgentModelsFailure::InvalidInput,
            Some("base_url"),
            "地址打不开"
        )
    );
    bed.agent.reply(2, "not json", "");
    assert_eq!(
        bed.fail(ModelsOp::Catalog).await.kind,
        AgentModelsFailure::Upstream
    );
    bed.agent.reply(3, "", "no such session");
    assert_eq!(
        bed.fail(ModelsOp::Catalog).await.kind,
        AgentModelsFailure::NotFound
    );
    bed.agent.reply(4, "", "dashboard down");
    let not_ready = bed.fail(ModelsOp::Catalog).await;
    assert_eq!(not_ready.kind, AgentModelsFailure::NotReady);
    assert_eq!(not_ready.detail.as_deref(), Some("dashboard down"));
    bed.agent.reply(9, "", "boom");
    let upstream = bed.fail(ModelsOp::Catalog).await;
    assert_eq!(upstream.kind, AgentModelsFailure::Upstream);
    assert!(upstream.detail.unwrap().contains('9'));
}

#[tokio::test]
async fn output_that_breaks_the_contract_is_refused_whole() {
    let bed = bed(MODELS).await;
    for stdout in [
        catalog("http://example.com/keys"),
        catalog("intent://x"),
        "{}".into(),
        "hi".into(),
    ] {
        bed.agent.reply(0, &stdout, "");
        assert_eq!(
            bed.fail(ModelsOp::Catalog).await.kind,
            AgentModelsFailure::Upstream,
            "{stdout}"
        );
    }
    let login = |url: &str, interval: u32| {
        json!({"session": "s1", "url": url, "code": "ABCD", "expires_in": 900, "interval": interval})
            .to_string()
    };
    let start = || ModelsOp::LoginStart {
        provider: "nous".into(),
        redirect_uri: None,
    };
    bed.agent
        .reply(0, &login("http://example.com/device", 5), "");
    let refused = bed.run::<ModelLogin>("alice", start()).await.unwrap_err();
    assert_eq!(refused.kind, AgentModelsFailure::Upstream);
    bed.agent
        .reply(0, &login("https://example.com/device", 0), "");
    let clamped: ModelLogin = bed.run("alice", start()).await.unwrap();
    assert_eq!(clamped.interval, wonton_protocol::MIN_LOGIN_POLL_SECS);
}

/// 服务商目录有几 KB：标准输出只抹 key、不截短，整份交出去。
#[tokio::test]
async fn a_catalog_longer_than_the_error_detail_comes_through_whole() {
    let bed = bed(MODELS).await;
    let providers: Vec<_> = (0..47)
        .map(|n| {
            json!({
                "id": format!("provider-{n}"), "name": format!("Provider {n}"),
                "recommended": n < 3,
                "auth": [{"type": "api_key", "key_url": format!("https://example.com/{n}/keys")}],
                "connected": false
            })
        })
        .collect();
    let stdout = json!({"providers": providers, "current": null}).to_string();
    assert!(stdout.len() > 4000);
    bed.agent.reply(0, &stdout, "");
    let catalog: ModelCatalog = bed.run("alice", ModelsOp::Catalog).await.unwrap();
    assert_eq!(catalog.providers.len(), 47);
    assert_eq!(catalog.providers[46].id, "provider-46");
}

#[tokio::test]
async fn each_reason_a_bot_cannot_take_models_is_told_apart() {
    let bed = bed(MODELS).await;
    let kind = |result: Result<_, AgentModelsError>| result.map(|_| ()).unwrap_err().kind;
    assert_eq!(
        kind(
            bed.manager
                .agent_models_target("app.missing", "alice", &langs())
        ),
        AgentModelsFailure::NotInstalled
    );
    assert_eq!(
        kind(bed.manager.agent_models_target(PLUGIN, "nobody", &langs())),
        AgentModelsFailure::NotInstalled
    );
    let manifest = support::manifest(MODELS);
    bed.manager
        .register_for_test(manifest, PluginStatus::Stopped, None);
    assert_eq!(
        kind(bed.manager.agent_models_target(PLUGIN, "alice", &langs())),
        AgentModelsFailure::NotReady
    );
    bed.manager
        .register_for_test(support::manifest(""), PluginStatus::Running, None);
    assert_eq!(
        kind(bed.manager.agent_models_target(PLUGIN, "alice", &langs())),
        AgentModelsFailure::Unsupported
    );
}

#[tokio::test]
async fn a_bot_whose_container_is_gone_is_not_ready() {
    let bed = bed(MODELS).await;
    let id = support::manifest(MODELS).id;
    bed.manager.bot_records_for_test(
        &id,
        &[("ghost", "@ghost:test", BotKeeper::Household, "bus")],
    );
    let target = bed
        .manager
        .agent_models_target(PLUGIN, "ghost", &langs())
        .unwrap();
    let error = bed
        .manager
        .exec_agent_models(&target, ModelsOp::Test, &langs())
        .await
        .unwrap_err();
    assert_eq!(error.kind, AgentModelsFailure::NotReady);
    assert!(bed.agent.seen().is_empty());
}

#[test]
fn the_manifest_only_takes_a_per_bot_models_command_on_agent_plugins() {
    let parse = |kind: &str, models: &str, bots: &str| {
        let yaml = format!(
            "services:\n  agent:\n    image: agent\n  web:\n    image: web\nx-wonton:\n  spec: \
             '0.17'\n  id: app.agent\n  version: 1.0.0\n  kind: {kind}\n  title: {{zh: 助手, en: \
             Agent}}\n  description: {{zh: 助手, en: Agent}}\n  agent: {{protocol: matrix, \
             models: {models}}}\n  permissions: [matrix.bot]\n{bots}"
        );
        PluginPackage::manifest_of(&yaml, &Languages::default())
            .map(|_| ())
            .map_err(|error| error.to_string())
    };
    let bots = "  bots: {services: [agent]}\n";
    assert!(parse("agent", "{service: agent, command: [m]}", bots).is_ok());
    assert!(parse("agent", "{service: web, command: [m]}", "").is_ok());
    for (kind, models, bots) in [
        ("app", "{service: agent, command: [m]}", bots),
        ("agent", "{service: agent, command: []}", bots),
        ("agent", "{service: agent, command: [' ']}", bots),
        ("agent", "{service: web, command: [m]}", bots),
        ("agent", "{service: agent, command: [m], extra: 1}", bots),
    ] {
        let error = parse(kind, models, bots).unwrap_err();
        assert!(
            error.contains("agent.models") || error.contains("extra"),
            "{kind} {models}: {error}"
        );
    }
}

/// 正好到上限的输出照常解析，多 1 字节就整份不要（引擎比上限多收 1 字节，分得清这两种）。
#[tokio::test]
async fn output_up_to_the_limit_parses_and_one_byte_more_is_refused() {
    let bed = bed(MODELS).await;
    let body = catalog("https://example.com/keys");
    // 不认识的字段插件可以带，拿它把输出垫到想要的长度；空白会被引擎去掉，不能用来垫。
    let padded = |len: usize| {
        let head = &body[..body.len() - 1];
        let pad = len - head.len() - r#","pad":""}"#.len();
        format!(r#"{head},"pad":"{}"}}"#, "x".repeat(pad))
    };
    assert_eq!(padded(MAX_MODELS_OUTPUT_BYTES).len(), MAX_MODELS_OUTPUT_BYTES);
    bed.agent.reply(0, &padded(MAX_MODELS_OUTPUT_BYTES), "");
    let whole: ModelCatalog = bed.run("alice", ModelsOp::Catalog).await.unwrap();
    assert_eq!(whole.providers.len(), 1);
    bed.agent.reply(0, &padded(MAX_MODELS_OUTPUT_BYTES + 1), "");
    assert_eq!(
        bed.fail(ModelsOp::Catalog).await.kind,
        AgentModelsFailure::Upstream
    );
}

/// 登录、交回回跳、确认过的选模型、测试：插件的回答原样交出，确认标记和回跳网址照样传给插件。
#[tokio::test]
async fn login_select_and_test_pass_the_plugins_answers_through() {
    let bed = bed(MODELS).await;
    bed.agent.reply(
        0,
        &json!({"session": "s1", "url": "https://example.com/device?code=AB", "code": "AB",
                "expires_in": 900, "interval": 5})
        .to_string(),
        "",
    );
    let start = ModelsOp::LoginStart {
        provider: "nous".into(),
        redirect_uri: None,
    };
    let login: ModelLogin = bed.run("alice", start).await.unwrap();
    assert_eq!((login.session.as_str(), login.interval), ("s1", 5));

    bed.agent.reply(0, r#"{"state": "approved"}"#, "");
    let callback = "wonton://model-login?code=c1&state=s";
    let finish = ModelsOp::LoginFinish {
        session: "s1".into(),
        callback_url: callback.into(),
    };
    let finished: ModelLoginState = bed.run("alice", finish).await.unwrap();
    assert_eq!(finished.state, LoginState::Approved);
    assert_eq!(bed.agent.seen().last().unwrap().input["callback_url"], callback);

    bed.agent.reply(
        0,
        r#"{"current": {"provider": "openrouter", "model": "big"}}"#,
        "",
    );
    let select = ModelsOp::Select {
        provider: "openrouter".into(),
        model: "big".into(),
        confirmed: true,
    };
    let selected: ModelSelection = bed.run("alice", select).await.unwrap();
    assert_eq!(selected.current.unwrap().model, "big");
    assert_eq!(bed.agent.seen().last().unwrap().input["confirmed"], true);

    bed.agent
        .reply(0, r#"{"result": "ok", "latency_ms": 800}"#, "");
    let tested: ModelTest = bed.run("alice", ModelsOp::Test).await.unwrap();
    assert_eq!(
        (tested.result, tested.latency_ms),
        (ModelTestResult::Ok, Some(800))
    );
}

fn package(agent_extra: &str, bots: bool) -> Result<(), String> {
    let bots = if bots {
        "  bots: {services: [agent]}\n"
    } else {
        ""
    };
    let yaml = format!(
        "services:\n  agent:\n    image: agent\nx-wonton:\n  spec: '0.17'\n  id: {PLUGIN}\n  \
         version: 1.0.0\n  kind: agent\n  title: {{zh: 助手, en: Agent}}\n  description: {{zh: 助手, \
         en: Agent}}\n  agent: {{protocol: matrix{agent_extra}}}\n  permissions: [matrix.bot]\n{bots}"
    );
    PluginPackage::parse(&yaml, &langs())
        .map(drop)
        .map_err(|e| e.to_string())
}

/// 模型命令的服务名要原样对得上 compose 里的服务；有没有 `bots` 都查。
#[test]
fn the_models_service_must_be_a_real_service() {
    package(MODELS, true).unwrap();
    package(MODELS, false).unwrap();
    let padded = ", models: {service: ' agent', command: [wonton-models]}";
    assert!(package(padded, true).is_err());
    let ghost = ", models: {service: ghost, command: [wonton-models]}";
    assert!(package(ghost, false).unwrap_err().contains("ghost"));
    assert!(package(ghost, true).is_err());
}
