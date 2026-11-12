//! 模型网关：在假 Docker 和假网关上验证凭据的签发、验证、对账和吊销，以及注入给使用方的变量。假网关的凭据命令
//! 按 `plugins/magpie/credentials.sh` 的约定：`issue` 已有同名凭据时不改动，`revoke` 本来没有也算成功。

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::http::{HeaderMap, StatusCode};
use axum::routing::get;
use tokio::net::TcpListener;

use super::super::fake_docker::FakeDocker;
use super::super::names::container_name;
use super::super::{PluginId, PluginManager, PluginPackage, PluginStatus};
use super::manager::manager_with;
use super::{env_of_container, package};
use crate::http::HttpClient;
use crate::matrix::Matrix;

type Keys = Arc<Mutex<BTreeMap<String, String>>>;

/// 假网关：`GET /v1/models` 只认签发过的凭据。
async fn fake_gateway(keys: Keys) -> u16 {
    let app = Router::new().route(
        "/v1/models",
        get(move |headers: HeaderMap| {
            let keys = keys.clone();
            async move {
                let bearer = headers
                    .get("authorization")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| v.strip_prefix("Bearer "))
                    .unwrap_or_default()
                    .to_owned();
                if keys.lock().unwrap().values().any(|k| *k == bearer) {
                    (StatusCode::OK, r#"{"data":[]}"#)
                } else {
                    (
                        StatusCode::UNAUTHORIZED,
                        r#"{"error":{"message":"unauthorized"}}"#,
                    )
                }
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
    PluginPackage::parse(&source).unwrap()
}

/// 一个使用方：唯一的服务拿到模型网关的全部变量。
fn consumer() -> String {
    package(
        "  web:\n    image: nginx\n    environment:\n      BASE: ${WONTON_MODEL_BASE_URL}\n      \
         ANTHROPIC: ${WONTON_MODEL_ANTHROPIC_BASE_URL}\n      KEY: ${WONTON_MODEL_API_KEY}\n      \
         MODEL: ${WONTON_MODEL_DEFAULT}",
        "  permissions: [models]",
    )
}

fn models() -> Vec<super::super::Permission> {
    vec!["models".parse().unwrap()]
}

struct Bed {
    docker: FakeDocker,
    keys: Keys,
    manager: PluginManager,
    _dir: tempfile::TempDir,
}

/// 装好假网关（预装系统插件的路径），凭据命令由假 Docker 按 magpie 脚本的约定回应。
async fn bed() -> Bed {
    let (docker, engine) = FakeDocker::start().await;
    let keys = Keys::default();
    let script = keys.clone();
    docker.on_exec(move |_, cmd, stdin| {
        let mut keys = script.lock().unwrap();
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
    });
    let port = fake_gateway(keys.clone()).await;
    let dir = tempfile::tempdir().unwrap();
    // 有 Bot 的使用方还要 Matrix。
    let (_tuwunel, config) = crate::matrix::fake::start().await;
    let matrix = Matrix::connect(&config, &dir.path().join("core"), HttpClient::new())
        .await
        .unwrap();
    let manager = manager_with(engine, &dir).with_matrix(Some(Arc::new(matrix)));
    manager
        .install_checked(provider(port), &[], false)
        .await
        .unwrap();
    Bed {
        docker,
        keys,
        manager,
        _dir: dir,
    }
}

fn web_env(bed: &Bed) -> BTreeMap<String, String> {
    env_of_container(&bed.docker, &container_name("app-test", "web"))
}

#[tokio::test]
async fn consumers_get_their_own_verified_credential_through_the_core() {
    let bed = bed().await;
    bed.manager.install(&consumer(), &models()).await.unwrap();
    let env = web_env(&bed);
    assert_eq!(env["BASE"], "http://models.wonton.internal:8080/v1");
    assert_eq!(env["ANTHROPIC"], "http://models.wonton.internal:8080");
    assert_eq!(env["MODEL"], "group/default");
    assert_eq!(bed.keys.lock().unwrap()["app.test"], env["KEY"]);
    assert_eq!(env["KEY"].len(), 64);
    // 值走标准输入，不进命令行参数。
    let issue = bed.docker.execs().into_iter().last().unwrap();
    assert_eq!(issue.container, container_name("system-models", "magpie"));
    assert!(!issue.cmd.iter().any(|arg| arg.contains(&env["KEY"])));
    assert_eq!(issue.stdin, format!("{}\n", env["KEY"]).into_bytes());
    // 核心直接跑在宿主上：插件经自己网络的网关访问 models.wonton.internal。
    let created = bed
        .docker
        .created(&container_name("app-test", "web"))
        .unwrap();
    let hosts = created["HostConfig"]["ExtraHosts"].clone();
    assert_eq!(
        hosts,
        serde_json::json!(["models.wonton.internal:172.30.0.1"])
    );
}

#[tokio::test]
async fn the_gateway_gets_lost_or_stale_credentials_back_from_the_core() {
    let bed = bed().await;
    bed.manager.install(&consumer(), &models()).await.unwrap();
    let key = web_env(&bed)["KEY"].clone();
    // 网关的数据丢了：对账时用同一个值重新签发，使用方不用重启。
    bed.keys.lock().unwrap().clear();
    assert!(bed.manager.reconcile_models().await.is_empty());
    assert_eq!(bed.keys.lock().unwrap()["app.test"], key);
    // 网关里的同名凭据不是这个值（issue 不改动已有的）：验证得到 401，先吊销再签发。
    bed.keys
        .lock()
        .unwrap()
        .insert("app.test".into(), "stale".into());
    assert!(bed.manager.reconcile_models().await.is_empty());
    assert_eq!(bed.keys.lock().unwrap()["app.test"], key);
}

#[tokio::test]
async fn uninstall_revokes_the_credential() {
    let bed = bed().await;
    let id = PluginId::parse("app.test").unwrap();
    bed.manager.install(&consumer(), &models()).await.unwrap();
    bed.manager.uninstall(&id, false).await.unwrap();
    assert!(bed.keys.lock().unwrap().is_empty(), "卸载时在网关里吊销");
    // 重装是一个新值：不删数据卸载也不保留凭据。
    bed.manager.install(&consumer(), &models()).await.unwrap();
    assert_eq!(bed.keys.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn consumers_need_a_gateway_that_accepts_the_credential() {
    // 没有网关：安装失败，说明是网关的原因。
    let (docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let m = manager_with(engine, &dir);
    let err = m.install(&consumer(), &models()).await.unwrap_err();
    assert_eq!(err.kind(), "models_unavailable", "{err}");
    assert!(docker.networks().is_empty());

    // 网关拒绝签发：错误里带上命令的标准错误，安装回滚。
    let bed = bed().await;
    bed.docker
        .on_exec(|_, cmd, _| match cmd.get(2).map(String::as_str) {
            Some("issue") => (1, "数据库被锁住了".to_owned()),
            _ => (0, String::new()),
        });
    let err = bed
        .manager
        .install(&consumer(), &models())
        .await
        .unwrap_err();
    assert_eq!(err.kind(), "models_unavailable", "{err}");
    assert!(err.to_string().contains("数据库被锁住了"), "{err}");
    assert_eq!(bed.manager.list().len(), 1, "只剩网关");
    let stored = bed._dir.path().join("plugins/model-keys/app.test.json");
    assert!(!stored.exists(), "没签发成功的凭据吊销后不留下");

    // 同一时间只能有一个网关。
    let port = fake_gateway(Keys::default()).await;
    let second = provider(port);
    let err = bed
        .manager
        .install_checked(second, &[], false)
        .await
        .unwrap_err();
    assert_eq!(err.kind(), "conflict", "{err}");
}

#[tokio::test]
async fn bot_services_get_the_bot_credential_and_shared_services_the_plugin_one() {
    let bed = bed().await;
    let source = package(
        "  gateway:\n    image: hermes\n    environment:\n      KEY: ${WONTON_MODEL_API_KEY}\n  \
         web:\n    image: nginx\n    environment:\n      KEY: ${WONTON_MODEL_API_KEY}",
        "  bot: {display_name: {zh: 测试, en: Test}}\n  bots: {services: [gateway]}\n  \
         permissions: [matrix.bot, models]",
    );
    let granted = vec!["matrix.bot".parse().unwrap(), "models".parse().unwrap()];
    bed.manager.install(&source, &granted).await.unwrap();
    let keys = bed.keys.lock().unwrap().clone();
    let gateway = env_of_container(&bed.docker, &container_name("app-test", "gateway"));
    assert_eq!(gateway["KEY"], keys["app.test/default"]);
    assert_eq!(web_env(&bed)["KEY"], keys["app.test"]);
    assert_ne!(keys["app.test"], keys["app.test/default"]);
}

#[tokio::test]
async fn a_failed_reconcile_marks_the_consumer_as_degraded() {
    let bed = bed().await;
    bed.manager.install(&consumer(), &models()).await.unwrap();
    bed.docker.on_exec(|_, _, _| (1, "网关坏了".to_owned()));
    let failures = bed.manager.reconcile_models().await;
    assert_eq!(failures.len(), 1);
    let consumer = bed
        .manager
        .list()
        .into_iter()
        .find(|p| p.id.as_str() == "app.test")
        .unwrap();
    let PluginStatus::Degraded { reason } = consumer.status else {
        panic!("对账失败的使用方应标为异常");
    };
    assert!(reason.contains("网关坏了"), "{reason}");
}

#[test]
fn the_hermes_and_magpie_drafts_render() {
    let plugins = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plugins");
    let magpie = PluginPackage::load_dir(&plugins.join("magpie")).unwrap();
    let spec = magpie.manifest.provides.clone().unwrap().models.unwrap();
    assert_eq!(spec.credentials_service(), "magpie");
    super::super::render::dry_run(&magpie).unwrap();
    // Hermes 的 Bot 服务拿到这个 Bot 的模型凭据和 Matrix 身份。
    let hermes = PluginPackage::load_dir(&plugins.join("hermes")).unwrap();
    let plan = super::super::render::dry_run(&hermes).unwrap();
    let gateway = plan
        .containers
        .iter()
        .find(|c| c.service == "gateway")
        .unwrap();
    let env = super::env_of(gateway);
    for expected in [
        "WONTON_MODEL_BASE_URL=http://models.wonton.internal/v1",
        "WONTON_MODEL_API_KEY=dry-run",
        "MATRIX_ACCESS_TOKEN=dry-run",
    ] {
        assert!(
            env.contains(&expected.to_owned()),
            "缺少 {expected}：{env:?}"
        );
    }
}
