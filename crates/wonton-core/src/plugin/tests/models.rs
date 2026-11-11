//! 模型网关：在假 Docker 和假网关上验证凭据的签发、验证、对账和吊销，以及注入给使用方的变量。假网关的凭据命令
//! 按 `plugins/magpie/credentials.sh` 的约定：`issue` 已有同名凭据时不改动，`revoke` 本来没有也算成功。

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::http::{HeaderMap, StatusCode};
use axum::routing::get;
use tokio::net::TcpListener;

use super::super::fake_docker::FakeDocker;
use super::super::names::Instance;
use super::super::names::container_name;
use super::super::{PluginId, PluginManager, PluginPackage, PluginStatus};
use super::github::zh;
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
    PluginPackage::parse(&source, &super::langs()).unwrap()
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
    let bed = bed_without_gateway().await;
    bed.install_gateway().await;
    bed
}

/// magpie 脚本的约定（`credentials.sh`）：`issue` 已有同名凭据时不改动，`revoke` 本来没有也算成功。
/// 失败注入把处理函数换掉之后，用它换回真的：issue 成功必须把值写进假网关的 `keys`。
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

/// 同 [`bed`]，但没有模型网关：声明 `models` 的插件就装在“没接上”的状态。
async fn bed_without_gateway() -> Bed {
    let (docker, engine) = FakeDocker::start().await;
    let keys = Keys::default();
    docker.on_exec(credential_script(keys.clone()));
    let dir = tempfile::tempdir().unwrap();
    // 有 Bot 的使用方还要 Matrix。
    let (_tuwunel, config) = crate::matrix::fake::start().await;
    let matrix = Matrix::connect(&config, &dir.path().join("core"), HttpClient::new())
        .await
        .unwrap();
    let manager = manager_with(engine, &dir).with_matrix(Some(Arc::new(matrix)));
    Bed {
        docker,
        keys,
        manager,
        _dir: dir,
    }
}

impl Bed {
    /// 装上假网关：装完核心就给没接上的使用方签发凭据（见安装完成的那条钩子）。
    async fn install_gateway(&self) {
        let port = fake_gateway(self.keys.clone()).await;
        self.manager
            .install_checked(provider(port), &[], &[], false, &super::langs())
            .await
            .unwrap();
    }

    /// 使用方详情页的轻提示。
    async fn hints(&self) -> wonton_protocol::PluginDetail {
        self.manager
            .judged_detail(&PluginId::parse("app.test").unwrap(), &zh())
            .await
            .unwrap()
    }
}

fn web_env(bed: &Bed) -> BTreeMap<String, String> {
    env_of_container(&bed.docker, &container_name("app-test", "web"))
}

#[tokio::test]
async fn consumers_get_their_own_verified_credential_through_the_core() {
    let bed = bed().await;
    bed.manager
        .install(&consumer(), &models(), super::langs())
        .await
        .unwrap();
    let env = web_env(&bed);
    assert_eq!(env["BASE"], "http://models.wonton.internal:8080/v1");
    assert_eq!(env["ANTHROPIC"], "http://models.wonton.internal:8080");
    assert_eq!(env["MODEL"], "group/default");
    assert_eq!(bed.keys.lock().unwrap()["app.test"], env["KEY"]);
    assert_eq!(env["KEY"].len(), 64);
    // 值走标准输入，不进命令行参数。健康探测凭据（<网关>/health）也在这之后签发，按名挑使用方的。
    let issue = bed
        .docker
        .execs()
        .into_iter()
        .rev()
        .find(|exec| exec.cmd.last().is_some_and(|name| name == "app.test"))
        .unwrap();
    assert_eq!(issue.container, container_name("system-models", "magpie"));
    assert!(!issue.cmd.iter().any(|arg| arg.contains(&env["KEY"])));
    assert_eq!(issue.stdin, format!("{}\n", env["KEY"]).into_bytes());
    // 核心直接跑在宿主上：插件经自己网络的网关访问 models.wonton.internal。
    let created = bed
        .docker
        .created(&container_name("app-test", "web"))
        .unwrap();
    let hosts = created["HostConfig"]["ExtraHosts"].clone();
    let gateway = bed
        .manager
        .engine
        .network_gateway("wonton-app-test")
        .await
        .unwrap();
    assert_eq!(
        hosts,
        serde_json::json!([format!("models.wonton.internal:{gateway}")])
    );
}

#[tokio::test]
async fn the_gateway_gets_lost_or_stale_credentials_back_from_the_core() {
    let bed = bed().await;
    bed.manager
        .install(&consumer(), &models(), super::langs())
        .await
        .unwrap();
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
    bed.manager
        .install(&consumer(), &models(), super::langs())
        .await
        .unwrap();
    bed.manager
        .uninstall(&id, false, &super::langs())
        .await
        .unwrap();
    // 健康探测凭据（<网关>/health）不属于使用方，不算数。
    let consumer_keys = |keys: &BTreeMap<String, String>| {
        keys.keys()
            .filter(|name| !name.ends_with("/health"))
            .count()
    };
    assert_eq!(
        consumer_keys(&bed.keys.lock().unwrap()),
        0,
        "卸载时在网关里吊销"
    );
    // 重装是一个新值：不删数据卸载也不保留凭据。
    bed.manager
        .install(&consumer(), &models(), super::langs())
        .await
        .unwrap();
    assert_eq!(consumer_keys(&bed.keys.lock().unwrap()), 1);
}

#[tokio::test]
async fn installs_without_a_gateway_and_the_variables_come_up_empty() {
    // 没有网关：照样装，状态照常 Running，模型变量全是空字符串。
    let (docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let m = manager_with(engine, &dir);
    m.install(&consumer(), &models(), super::langs())
        .await
        .unwrap();
    let env = env_of_container(&docker, &container_name("app-test", "web"));
    for name in ["BASE", "ANTHROPIC", "KEY", "MODEL"] {
        assert_eq!(env[name], "", "{name}");
    }
    // 详情页有“没接上”的轻提示，没有签发失败的原因可带。
    let detail = m
        .judged_detail(&PluginId::parse("app.test").unwrap(), &zh())
        .await
        .unwrap();
    assert_eq!(detail.hints.len(), 1);
    assert_eq!(
        detail.hints[0].category,
        wonton_protocol::PluginHintCategory::ModelsNotConnected
    );
    assert_eq!(detail.hints[0].detail, None);
}

#[tokio::test]
async fn the_gateway_arriving_later_issues_and_rebuilds_the_consumer() {
    let bed = bed_without_gateway().await;
    bed.manager
        .install(&consumer(), &models(), super::langs())
        .await
        .unwrap();
    assert_eq!(web_env(&bed)["BASE"], "");

    // 网关装上：核心给没接上的使用方签发凭据、按新变量重建容器，轻提示消失。
    bed.install_gateway().await;
    let env = web_env(&bed);
    assert_eq!(env["BASE"], "http://models.wonton.internal:8080/v1");
    assert_eq!(bed.keys.lock().unwrap()["app.test"], env["KEY"]);
    assert!(bed.hints().await.hints.is_empty());
}

#[tokio::test]
async fn the_gateway_coming_back_again_leaves_connected_consumers_alone() {
    let bed = bed().await;
    bed.manager
        .install(&consumer(), &models(), super::langs())
        .await
        .unwrap();
    let env = web_env(&bed);
    let web = container_name("app-test", "web");
    assert_eq!(bed.docker.creations(&web), 1);

    // 网关再就绪一次（核心重启、网关容器恢复、升级都对一次账）：已接上的使用方
    // 凭据值没变，容器不动，不白重启一遍。
    assert!(bed.manager.reconcile_models().await.is_empty());
    assert_eq!(bed.docker.creations(&web), 1);
    assert_eq!(web_env(&bed), env);
}

#[tokio::test]
async fn a_failed_issue_still_installs_and_the_hint_carries_the_reason() {
    let bed = bed().await;
    bed.docker
        .on_exec(|_, cmd, _| match cmd.get(2).map(String::as_str) {
            Some("issue") => (1, "数据库被锁住了".to_owned()),
            _ => (0, String::new()),
        });
    // 签发失败：照样装，状态照常，变量为空。
    bed.manager
        .install(&consumer(), &models(), super::langs())
        .await
        .unwrap();
    assert_eq!(web_env(&bed)["KEY"], "");
    let consumer = bed
        .manager
        .list()
        .into_iter()
        .find(|p| p.id.as_str() == "app.test")
        .unwrap();
    assert!(
        matches!(consumer.status, PluginStatus::Running),
        "签发失败不改变插件状态"
    );
    // 轻提示带上签发失败的原因；记下的文件里只有原因，没有凭据值。
    let detail = bed.hints().await;
    assert_eq!(detail.hints.len(), 1);
    let reason = detail.hints[0].detail.as_deref().unwrap();
    assert!(reason.contains("数据库被锁住了"), "{reason}");
    let stored =
        std::fs::read_to_string(bed._dir.path().join("plugins/model-keys/app.test.json")).unwrap();
    assert!(stored.contains("数据库被锁住了"), "{stored}");

    // 网关修好：对账时签发、按新变量重建，提示消失。
    bed.docker.on_exec(credential_script(bed.keys.clone()));
    let failures = bed.manager.reconcile_models().await;
    assert!(failures.is_empty(), "{failures:?}");
    assert_eq!(
        web_env(&bed)["BASE"],
        "http://models.wonton.internal:8080/v1"
    );
    assert!(bed.hints().await.hints.is_empty());
}

#[tokio::test]
async fn uninstalling_the_gateway_leaves_the_consumer_running_and_the_hint_returns() {
    let bed = bed().await;
    bed.manager
        .install(&consumer(), &models(), super::langs())
        .await
        .unwrap();
    let key = web_env(&bed)["KEY"].clone();

    // 网关卸掉：使用方不动，还在运行，轻提示变回“没接上”。
    bed.manager
        .uninstall(
            &PluginId::parse("system.models").unwrap(),
            false,
            &super::langs(),
        )
        .await
        .unwrap();
    let consumer = bed
        .manager
        .list()
        .into_iter()
        .find(|p| p.id.as_str() == "app.test")
        .unwrap();
    assert!(matches!(consumer.status, PluginStatus::Running));
    assert_eq!(web_env(&bed)["KEY"], key, "使用方的容器没动");
    let detail = bed.hints().await;
    assert_eq!(detail.hints.len(), 1);
    assert_eq!(
        detail.hints[0].category,
        wonton_protocol::PluginHintCategory::ModelsNotConnected
    );
    // 网关不在：对账不碰使用方。
    assert!(bed.manager.reconcile_models().await.is_empty());

    // 网关装回来：凭据值还记着，对账用同一个值重新签发，容器不用重建就接上了。
    bed.install_gateway().await;
    assert_eq!(web_env(&bed)["KEY"], key, "重新签发用记下的同一个值");
    assert!(bed.hints().await.hints.is_empty());
}

#[tokio::test]
async fn a_rebuild_keeps_the_recorded_vars_even_when_the_gateway_is_gone() {
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
    let port = fake_gateway(keys).await;
    let dir = tempfile::tempdir().unwrap();
    let manager = Arc::new(manager_with(engine, &dir));
    manager
        .install_checked(provider(port), &[], &[], false, &super::langs())
        .await
        .unwrap();
    manager
        .install(&consumer(), &models(), super::langs())
        .await
        .unwrap();
    let env = env_of_container(&docker, &container_name("app-test", "web"));

    // 网关不在时重建使用方（改资源上限）：注入记下的变量，不退回空值。
    manager
        .uninstall(
            &PluginId::parse("system.models").unwrap(),
            false,
            &super::langs(),
        )
        .await
        .unwrap();
    let detail = manager
        .set_limits(
            PluginId::parse("app.test").unwrap(),
            wonton_protocol::PluginLimits {
                memory: Some(1024 * 1024 * 1024),
                cpu_percent: None,
            },
            zh(),
        )
        .await
        .unwrap();
    let rebuilt = env_of_container(&docker, &container_name("app-test", "web"));
    assert_eq!(rebuilt["KEY"], env["KEY"], "记下的凭据原样注入");
    assert_eq!(rebuilt["BASE"], env["BASE"]);
    assert_eq!(detail.hints.len(), 1, "网关不在，重建后轻提示照给");
}

#[tokio::test]
async fn a_record_without_bot_keys_still_reads_and_the_plugin_comes_up() {
    let bed = bed().await;
    bed.manager
        .install(&consumer(), &models(), super::langs())
        .await
        .unwrap();
    let env = web_env(&bed);
    // 较早的核心写下的记录：`vars` 里没有之后才加的 `bot_keys`。
    let path = bed._dir.path().join("plugins/model-keys/app.test.json");
    let mut stored: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    stored["vars"].as_object_mut().unwrap().remove("bot_keys");
    std::fs::write(&path, stored.to_string()).unwrap();

    // 核心重启：同一份数据目录和容器。
    let restarted = manager_with(bed.manager.engine.clone(), &bed._dir);
    restarted.restore().await.unwrap();
    let status = |m: &PluginManager| {
        m.list()
            .into_iter()
            .find(|p| p.id.as_str() == "app.test")
            .unwrap()
            .status
    };
    assert!(matches!(status(&restarted), PluginStatus::Running));

    // 网关就绪后对账：旧记录能读出来，不把插件标成异常，也不白重建容器。
    let failures = restarted.reconcile_models().await;
    assert!(failures.is_empty(), "{failures:?}");
    assert!(matches!(status(&restarted), PluginStatus::Running));
    assert_eq!(web_env(&bed), env);
    let detail = restarted
        .judged_detail(&PluginId::parse("app.test").unwrap(), &zh())
        .await
        .unwrap();
    assert!(detail.hints.is_empty());
}

#[tokio::test]
async fn only_one_gateway_can_be_installed() {
    let bed = bed().await;
    let port = fake_gateway(Keys::default()).await;
    let err = bed
        .manager
        .install_checked(provider(port), &[], &[], false, &super::langs())
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
    bed.manager
        .install(&source, &granted, super::langs())
        .await
        .unwrap();
    let keys = bed.keys.lock().unwrap().clone();
    // gateway 是 Bot 服务：容器按 Bot 一份。
    let gateway = env_of_container(
        &bed.docker,
        &Instance::default().bot_container("app-test", "gateway", "default"),
    );
    assert_eq!(gateway["KEY"], keys["app.test/default"]);
    assert_eq!(web_env(&bed)["KEY"], keys["app.test"]);
    assert_ne!(keys["app.test"], keys["app.test/default"]);
}

#[tokio::test]
async fn a_failed_reconcile_marks_the_consumer_as_degraded() {
    let bed = bed().await;
    bed.manager
        .install(&consumer(), &models(), super::langs())
        .await
        .unwrap();
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
