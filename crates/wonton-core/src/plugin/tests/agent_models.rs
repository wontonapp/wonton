//! Agent 插件的模型接入（ADR-0129）：核心按 Bot 执行声明的命令，查过输出才转出，key 不回显。

use serde_json::json;
use wonton_protocol::{
    AgentModelsFailure, BotKeeper, KeyCheck, KeyCheckResult, ModelCatalog, ModelLogin, ModelsOp,
    SecretString,
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
    let secret = "sk-or-v1-0123456789";
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
