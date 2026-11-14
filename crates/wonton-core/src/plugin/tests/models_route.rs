//! 默认路由探测的行为测试：给定网关行为，断言详情页的轻提示和补全请求的次数。
//! 插件状态不因模型问题改变（ADR-0055：状态由容器健康决定）；补全的计费节奏见 `model_health`。

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::http::{HeaderMap, StatusCode};
use axum::routing::{get, post};
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio::time::{Duration, Instant};

use super::super::fake_docker::FakeDocker;
use super::super::names::container_name;
use super::super::{PluginId, PluginManager, PluginPackage, PluginStatus};
use super::manager::manager_with;
use super::{langs, package};

type Keys = Arc<Mutex<BTreeMap<String, String>>>;

#[derive(Clone)]
struct RouteReply {
    listed: bool,
    catalog_status: StatusCode,
    status: StatusCode,
    body: Value,
    catalogs: usize,
    completions: usize,
}

impl Default for RouteReply {
    fn default() -> Self {
        Self {
            listed: false,
            catalog_status: StatusCode::OK,
            status: StatusCode::OK,
            body: json!({
                "choices": [{"message": {"role": "assistant", "content": "ok"}, "finish_reason": "stop"}]
            }),
            catalogs: 0,
            completions: 0,
        }
    }
}

type RouteState = Arc<Mutex<RouteReply>>;

/// 假网关：`GET /v1/models` 只认签发过的凭据；补全按脚本的状态和正文回答并计数。
async fn gateway_with_route(keys: Keys, state: RouteState) -> u16 {
    let completions = state.clone();
    let app = Router::new()
        .route(
            "/v1/models",
            get(move |headers: HeaderMap| {
                let keys = keys.clone();
                let state = state.clone();
                async move {
                    let bearer = headers
                        .get("authorization")
                        .and_then(|v| v.to_str().ok())
                        .and_then(|v| v.strip_prefix("Bearer "))
                        .unwrap_or_default()
                        .to_owned();
                    if !keys.lock().unwrap().values().any(|k| *k == bearer) {
                        return (
                            StatusCode::UNAUTHORIZED,
                            r#"{"error":{"message":"unauthorized"}}"#.to_owned(),
                        );
                    }
                    state.lock().unwrap().catalogs += 1;
                    let reply = state.lock().unwrap().clone();
                    if reply.catalog_status != StatusCode::OK {
                        return (reply.catalog_status, String::new());
                    }
                    let data = if reply.listed {
                        json!([{"id": "group/default"}])
                    } else {
                        json!([])
                    };
                    (StatusCode::OK, json!({"data": data}).to_string())
                }
            }),
        )
        .route(
            "/v1/chat/completions",
            post(move |request: axum::Json<Value>| {
                let state = completions.clone();
                async move {
                    assert_eq!(request["model"], "group/default");
                    let reply = {
                        let mut state = state.lock().unwrap();
                        state.completions += 1;
                        state.clone()
                    };
                    (reply.status, axum::Json(reply.body))
                }
            }),
        );
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    port
}

fn provider(port: u16) -> PluginPackage {
    let source = format!(
        "services:\n  magpie:\n    image: magpie\nx-wonton:\n  spec: '0.4'\n  id: system.models\n  version: 1.0.0\n  \
         kind: system\n  title: {{zh: 模型, en: Models}}\n  description: {{zh: 模型, en: Models}}\n  \
         provides:\n    models:\n      service: magpie\n      port: {port}\n      \
         apis: {{openai: /v1, anthropic: /}}\n      default_model: group/default\n      credentials:\n        \
         issue: [sh, /wonton/credentials.sh, issue, '{{name}}']\n        \
         revoke: [sh, /wonton/credentials.sh, revoke, '{{name}}']\n  permissions: []"
    );
    PluginPackage::parse(&source, &langs()).unwrap()
}

/// 一个使用方：唯一的服务拿到模型网关的全部变量。
fn consumer() -> String {
    package(
        "  web:\n    image: nginx\n    environment:\n      BASE: ${WONTON_MODEL_BASE_URL}\n      \
         KEY: ${WONTON_MODEL_API_KEY}\n      MODEL: ${WONTON_MODEL_DEFAULT}",
        "  permissions: [models]",
    )
}

fn models() -> Vec<super::super::Permission> {
    vec!["models".parse().unwrap()]
}

/// magpie 脚本的约定：`issue` 已有同名凭据时不改动，`revoke` 本来没有也算成功。
fn credential_script(
    keys: Keys,
) -> impl Fn(&str, &[String], &[u8]) -> (i64, String) + Send + Sync + 'static {
    move |_, cmd, stdin| {
        let mut keys = keys.lock().unwrap();
        let value = String::from_utf8_lossy(stdin).trim().to_owned();
        match cmd {
            [_, _, action, name] if action == "issue" => {
                keys.entry(name.clone()).or_insert(value);
                (0, String::new())
            }
            [_, _, action, name] if action == "revoke" => {
                keys.remove(name);
                (0, String::new())
            }
            _ => (2, "用法不对".to_owned()),
        }
    }
}

struct Bed {
    docker: FakeDocker,
    engine: super::super::docker::Engine,
    manager: PluginManager,
    _dir: tempfile::TempDir,
}

async fn bed_with_route(state: RouteState) -> Bed {
    let (docker, engine) = FakeDocker::start().await;
    let keys = Keys::default();
    docker.on_exec(credential_script(keys.clone()));
    let port = gateway_with_route(keys.clone(), state).await;
    let dir = tempfile::tempdir().unwrap();
    let manager = manager_with(engine.clone(), &dir);
    manager
        .install_checked(
            provider(port),
            &[],
            &[],
            false,
            Default::default(),
            &langs(),
        )
        .await
        .unwrap();
    Bed {
        docker,
        engine,
        manager,
        _dir: dir,
    }
}

impl Bed {
    async fn install_consumer(&self) {
        self.manager
            .install(&consumer(), &models(), langs())
            .await
            .unwrap();
    }

    async fn hints(&self) -> Vec<wonton_protocol::PluginHint> {
        self.manager
            .judged_detail(&PluginId::parse("app.test").unwrap(), &super::github::zh())
            .await
            .unwrap()
            .hints
    }

    fn consumer_status(&self) -> PluginStatus {
        self.manager
            .list()
            .into_iter()
            .find(|p| p.id.as_str() == "app.test")
            .unwrap()
            .status
    }

    fn consumer_id() -> PluginId {
        PluginId::parse("app.test").unwrap()
    }

    fn assert_running(&self) {
        assert!(
            matches!(self.consumer_status(), PluginStatus::Running),
            "模型问题不改变插件状态"
        );
    }
}

#[tokio::test]
async fn model_settings_uses_stored_consumer_state_without_probing() {
    let state = Arc::new(Mutex::new(RouteReply {
        listed: true,
        ..RouteReply::default()
    }));
    let bed = bed_with_route(state.clone()).await;
    let before = state.lock().unwrap().catalogs;
    let settings = bed.manager.model_settings(&langs()).await.unwrap();
    assert!(!settings.has_configured_models);
    assert_eq!(state.lock().unwrap().catalogs, before);

    bed.install_consumer().await;
    let before = state.lock().unwrap().catalogs;
    let settings = bed.manager.model_settings(&langs()).await.unwrap();
    assert!(settings.has_configured_models);
    assert_eq!(state.lock().unwrap().catalogs, before);
}

#[tokio::test]
async fn missing_default_model_hints_and_the_plugin_stays_running() {
    let state = Arc::new(Mutex::new(RouteReply::default()));
    let bed = bed_with_route(state.clone()).await;
    bed.install_consumer().await;
    // 缺模型时不发补全；详情页给“没有默认模型”的轻提示，状态照常。
    assert_eq!(state.lock().unwrap().completions, 0);
    let hints = bed.hints().await;
    assert_eq!(hints.len(), 1);
    assert_eq!(
        hints[0].category,
        wonton_protocol::PluginHintCategory::ModelsNoDefaultModel
    );
    assert_eq!(hints[0].detail, None);
    bed.assert_running();

    // 默认模型出现：下一轮巡检只读列表就把提示清掉，不为路由出现补发补全。
    state.lock().unwrap().listed = true;
    let now = Instant::now();
    bed.manager
        .watch_models_at(
            vec![Bed::consumer_id()],
            false,
            now + Duration::from_secs(30),
        )
        .await;
    assert_eq!(state.lock().unwrap().completions, 0);
    assert!(bed.hints().await.is_empty());
    bed.assert_running();
}

#[tokio::test]
async fn healthy_rounds_only_read_the_catalog() {
    let state = Arc::new(Mutex::new(RouteReply {
        listed: true,
        ..RouteReply::default()
    }));
    let bed = bed_with_route(state.clone()).await;
    bed.install_consumer().await;
    // 刚安装的那一次补全；之后多轮健康巡检只读列表。
    assert_eq!(state.lock().unwrap().completions, 1);
    assert!(bed.hints().await.is_empty());
    let catalogs = state.lock().unwrap().catalogs;
    let now = Instant::now();
    for round in 1..=12 {
        bed.manager
            .watch_models_at(
                vec![Bed::consumer_id()],
                false,
                now + Duration::from_secs(round * 30),
            )
            .await;
    }
    assert_eq!(state.lock().unwrap().completions, 1, "健康稳态不发补全");
    assert_eq!(state.lock().unwrap().catalogs, catalogs + 12);
    bed.assert_running();
}

#[tokio::test]
async fn provider_error_hints_with_detail_and_retries_every_five_minutes() {
    let state = Arc::new(Mutex::new(RouteReply {
        listed: true,
        status: StatusCode::UNAUTHORIZED,
        ..RouteReply::default()
    }));
    let bed = bed_with_route(state.clone()).await;
    bed.install_consumer().await;
    // 刚安装的一次补全被拒：轻提示带出原因，状态照常。
    assert_eq!(state.lock().unwrap().completions, 1);
    let hints = bed.hints().await;
    assert_eq!(hints.len(), 1);
    assert_eq!(
        hints[0].category,
        wonton_protocol::PluginHintCategory::ModelsProviderError
    );
    assert!(hints[0].detail.as_deref().unwrap().contains("401"));
    bed.assert_running();

    let start = Instant::now();
    let id = Bed::consumer_id();
    for seconds in [30, 60, 299] {
        bed.manager
            .watch_models_at(
                vec![id.clone()],
                false,
                start + Duration::from_secs(seconds),
            )
            .await;
    }
    assert_eq!(state.lock().unwrap().completions, 1, "复查没到点不补发");
    assert_eq!(bed.hints().await.len(), 1);
    bed.manager
        .watch_models_at(vec![id.clone()], false, start + Duration::from_secs(300))
        .await;
    assert_eq!(state.lock().unwrap().completions, 2);

    // 服务商恢复：五分钟到点复查成功，提示清掉，之后不再发。
    state.lock().unwrap().status = StatusCode::OK;
    bed.manager
        .watch_models_at(vec![id.clone()], false, start + Duration::from_secs(599))
        .await;
    assert_eq!(state.lock().unwrap().completions, 2);
    assert_eq!(bed.hints().await.len(), 1);
    bed.manager
        .watch_models_at(vec![id.clone()], false, start + Duration::from_secs(600))
        .await;
    assert_eq!(state.lock().unwrap().completions, 3);
    assert!(bed.hints().await.is_empty());
    bed.assert_running();
    for seconds in [630, 900, 1200] {
        bed.manager
            .watch_models_at(
                vec![id.clone()],
                false,
                start + Duration::from_secs(seconds),
            )
            .await;
    }
    assert_eq!(state.lock().unwrap().completions, 3);
}

#[tokio::test]
async fn unreachable_model_list_reads_as_not_connected() {
    let state = Arc::new(Mutex::new(RouteReply {
        listed: true,
        catalog_status: StatusCode::INTERNAL_SERVER_ERROR,
        ..RouteReply::default()
    }));
    let bed = bed_with_route(state.clone()).await;
    bed.install_consumer().await;
    // 列表都拿不到：事实上没接上，归 models_not_connected（ADR-0055 没写这一类，PR 里说明）。
    let hints = bed.hints().await;
    assert_eq!(hints.len(), 1);
    assert_eq!(
        hints[0].category,
        wonton_protocol::PluginHintCategory::ModelsNotConnected
    );
    assert!(hints[0].detail.is_some());
    bed.assert_running();

    // 列表回来：签发对上账、探测通过，提示清掉。
    state.lock().unwrap().catalog_status = StatusCode::OK;
    let failures = bed.manager.reconcile_models().await;
    assert!(failures.is_empty(), "{failures:?}");
    bed.manager
        .watch_models_at(vec![Bed::consumer_id()], true, Instant::now())
        .await;
    assert!(bed.hints().await.is_empty());
    bed.assert_running();

    // 网关停了：详情页现场推导成“没接上”，没有原始原因可带。
    bed.manager.set(
        &PluginId::parse("system.models").unwrap(),
        PluginStatus::Stopped,
        None,
    );
    bed.manager
        .watch_models_at(vec![Bed::consumer_id()], false, Instant::now())
        .await;
    let hints = bed.hints().await;
    assert_eq!(hints.len(), 1);
    assert_eq!(
        hints[0].category,
        wonton_protocol::PluginHintCategory::ModelsNotConnected
    );
    assert_eq!(hints[0].detail, None);
    bed.assert_running();
}

#[tokio::test]
async fn starting_a_consumer_probes_once_but_an_idempotent_start_does_not() {
    let state = Arc::new(Mutex::new(RouteReply {
        listed: true,
        ..RouteReply::default()
    }));
    let bed = bed_with_route(state.clone()).await;
    bed.install_consumer().await;
    let manager = Arc::new(bed.manager);
    let id = Bed::consumer_id();
    assert_eq!(state.lock().unwrap().completions, 1);
    manager.start_detached(id.clone()).await.unwrap();
    assert_eq!(state.lock().unwrap().completions, 1, "已经在跑就不重发");

    // 容器停了再启动：算“刚启动”，补全一次；之后的巡检不再发。
    let container = container_name("app-test", "web");
    bed.docker.set_state(
        &container,
        json!({"Status": "exited", "Running": false, "ExitCode": 0}),
    );
    manager.watch_containers().await;
    assert!(matches!(
        manager.status_of(&id),
        Some(PluginStatus::Degraded { .. })
    ));
    bed.docker.set_start_state(
        &container,
        json!({"Status": "running", "Running": true, "Health": {"Status": "healthy"}}),
    );
    manager.start_detached(id.clone()).await.unwrap();
    assert_eq!(state.lock().unwrap().completions, 2);
    manager.watch_containers().await;
    assert_eq!(state.lock().unwrap().completions, 2);
}

#[tokio::test]
async fn a_restarted_core_probes_once_then_uses_only_catalogs() {
    let state = Arc::new(Mutex::new(RouteReply {
        listed: true,
        ..RouteReply::default()
    }));
    let bed = bed_with_route(state.clone()).await;
    bed.install_consumer().await;
    assert_eq!(state.lock().unwrap().completions, 1);
    let restarted = manager_with(bed.engine.clone(), &bed._dir);
    restarted.restore().await.unwrap();
    // 核心启动首轮：补全一次；之后的巡检只读列表。
    restarted.watch_containers().await;
    assert_eq!(state.lock().unwrap().completions, 2);
    restarted.watch_containers().await;
    assert_eq!(state.lock().unwrap().completions, 2);
}

#[tokio::test]
async fn subscription_agent_without_models_permission_is_left_alone() {
    let state = Arc::new(Mutex::new(RouteReply::default()));
    let bed = bed_with_route(state.clone()).await;
    let source = package(
        "  agent:\n    image: agent\n",
        "  agent: {protocol: acp}\n  permissions: []",
    )
    .replace("kind: app", "kind: agent");
    bed.manager.install(&source, &[], langs()).await.unwrap();
    let id = PluginId::parse("app.test").unwrap();
    assert!(matches!(
        bed.manager.status_of(&id),
        Some(PluginStatus::Running)
    ));
    bed.manager.watch_containers().await;
    // 订阅账号直连的 Agent 不受网关影响：不探测、没有提示、状态照常。
    assert_eq!(state.lock().unwrap().completions, 0);
    let detail = bed
        .manager
        .judged_detail(&id, &super::github::zh())
        .await
        .unwrap();
    assert!(detail.hints.is_empty());
    assert!(matches!(
        bed.manager.status_of(&id),
        Some(PluginStatus::Running)
    ));
}

#[tokio::test]
async fn preview_warns_when_default_model_is_missing_and_goes_quiet_when_listed() {
    let state = Arc::new(Mutex::new(RouteReply::default()));
    let bed = bed_with_route(state.clone()).await;
    let parsed = PluginPackage::parse(&consumer(), &langs()).unwrap();
    let preview = bed
        .manager
        .preview_install(&parsed, &langs())
        .await
        .unwrap();
    assert!(preview.blocked.is_none());
    assert!(
        preview
            .notices
            .iter()
            .any(|n| n.emphasis && n.text.contains("Connect a model provider")),
        "{:?}",
        preview.notices
    );

    // 模型接上：预览安静下来。
    state.lock().unwrap().listed = true;
    let preview = bed
        .manager
        .preview_install(&parsed, &langs())
        .await
        .unwrap();
    assert!(
        !preview
            .notices
            .iter()
            .any(|n| n.text.contains("Connect a model provider")),
        "{:?}",
        preview.notices
    );

    // 不申请 models 的普通应用：不提示。
    state.lock().unwrap().listed = false;
    let plain = PluginPackage::parse(
        &package("  web:\n    image: nginx\n", "  permissions: []"),
        &langs(),
    )
    .unwrap();
    let preview = bed.manager.preview_install(&plain, &langs()).await.unwrap();
    assert!(
        !preview
            .notices
            .iter()
            .any(|n| n.text.contains("Connect a model provider")),
        "{:?}",
        preview.notices
    );
}
