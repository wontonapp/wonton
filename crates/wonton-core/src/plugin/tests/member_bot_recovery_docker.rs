//! 真实 Docker：撤权后 Bot 的命名卷和匿名卷都留着，核心重启后不被自动起起来，重试接回同一批卷、
//! 数据原样。失败原因如实报出来。

use std::sync::Arc;
use std::time::Duration;

use bollard::models::ContainerStateStatusEnum;

use super::super::docker::Engine;
use super::super::{PluginId, PluginStatus};
use super::manager::manager_in_instance;
use crate::config::DEFAULT_INSTALL_TIMEOUT;
use crate::http::HttpClient;
use crate::matrix::Matrix;
use crate::matrix::fake::start;
use crate::test_docker::Names;

/// busybox：本地从 ECR Public 拉；CI 经 `WONTON_TEST_IMAGES` 指到镜像仓库副本（wonton-test-images）。
fn image() -> String {
    wonton_test_images::image(
        "public.ecr.aws/docker/library/busybox@sha256:a7dbc319b1438115a207e11930dc4bb3e643dcb3ec0bdd694bb36cd8fad289ec",
    )
}

fn id() -> PluginId {
    PluginId::parse("app.vol").unwrap()
}

fn package() -> String {
    format!(
        "services:
  bot:
    image: {}
    init: true
    command: ['sh', '-c', 'sleep infinity']
    volumes: ['data:/named', '/anon']
    environment:
      BOT: ${{WONTON_BOT_NAME}}
      TOKEN: ${{WONTON_MATRIX_ACCESS_TOKEN}}
volumes:
  data:
x-wonton:
  spec: '0.5'
  id: app.vol
  version: 1.0.0
  kind: agent
  title: {{zh: 测试, en: Test}}
  description: {{zh: 测试, en: Test}}
  bot: {{display_name: {{zh: 回声, en: Echo}}}}
  agent: {{protocol: matrix}}
  bots: {{services: [bot]}}
  permissions: [matrix.bot]
",
        image()
    )
}

async fn read(engine: &Engine, container: &str, path: &str) -> String {
    engine
        .exec_output(
            container,
            &["cat".to_owned(), path.to_owned()],
            None,
            Duration::from_secs(30),
        )
        .await
        .unwrap()
        .stdout
        .trim()
        .to_owned()
}

async fn write(engine: &Engine, container: &str, path: &str, text: &str) {
    engine
        .exec_output(
            container,
            &[
                "sh".to_owned(),
                "-c".to_owned(),
                format!("echo {text} > {path}"),
            ],
            None,
            Duration::from_secs(30),
        )
        .await
        .unwrap();
}

/// 清掉这套实例留下的容器和网络：共用主机慢时 Docker 建容器的请求会超时，而服务端还在建——列表拿到空
/// 也要再等一会儿确认，免得下一次创建撞上正在出现的同名容器。
async fn sweep(engine: &Engine, instance: &crate::plugin::names::Instance, id: &PluginId) {
    for _ in 0..12 {
        let Ok(found) = engine
            .labeled(crate::plugin::names::LABEL_PLUGIN, id.as_str(), |labels| {
                instance.owns(labels)
            })
            .await
        else {
            return;
        };
        if found.containers.is_empty() && found.networks.is_empty() {
            tokio::time::sleep(Duration::from_secs(5)).await;
            let again = engine
                .labeled(crate::plugin::names::LABEL_PLUGIN, id.as_str(), |labels| {
                    instance.owns(labels)
                })
                .await;
            if again.is_ok_and(|again| again.containers.is_empty()) {
                return;
            }
            continue;
        }
        for container in &found.containers {
            let _ = engine.remove_container(container).await;
        }
        for network in &found.networks {
            let _ = engine.remove_network(network).await;
        }
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
}

async fn running(engine: &Engine, container: &str) -> bool {
    engine
        .inspect_container(container)
        .await
        .unwrap()
        .is_some_and(|info| {
            info.state.and_then(|state| state.status) == Some(ContainerStateStatusEnum::RUNNING)
        })
}

#[tokio::test]
#[ignore = "需要 Docker，撤权后命名卷和匿名卷都留着，重启不被起，重试接回同一批卷"]
#[allow(clippy::too_many_lines)] // 一条线走完：装→写两种卷→撤权→重启→重试→核对
async fn a_restricted_bot_keeps_both_kinds_of_volume_across_a_restart() {
    // 共用主机满载时建容器能超过普通请求的 120 秒：这个用例放宽到 10 分钟。
    let engine = Engine::connect()
        .unwrap()
        .with_request_limit_for_test(Duration::from_secs(600));
    engine.check_server().await.unwrap();
    let names = Names::new();
    let dir = tempfile::tempdir().unwrap();
    let (tuwunel, config) = start().await;
    let matrix = Arc::new(
        Matrix::connect(&config, &dir.path().join("core"), HttpClient::new())
            .await
            .unwrap(),
    );
    let manager = |engine: Engine| {
        let mut manager = manager_in_instance(engine, &dir, names.name());
        manager.cfg.install_timeout = DEFAULT_INSTALL_TIMEOUT;
        manager.with_matrix(Some(matrix.clone()))
    };
    let container = names.instance().bot_container("app-vol", "bot", "alice");
    // 共用主机忙时 Docker 建容器可能超过单次请求时限（120 秒），装失败就清掉残留重来；失败的管理器还占着
    // 插件表，每次用新的。
    let granted = vec!["matrix.bot".parse().unwrap()];
    let mut tries = 0;
    let m = loop {
        sweep(&engine, names.instance(), &id()).await;
        let m = manager(engine.clone());
        match m.install(&package(), &granted, super::langs()).await {
            Ok(_) => break m,
            Err(error) => {
                tries += 1;
                assert!(tries < 4, "装了三次都没成：{error}");
            }
        }
    };
    m.add_bot(&id(), "alice", alice()).await.unwrap();
    write(&engine, &container, "/named/f", "named-before").await;
    write(&engine, &container, "/anon/f", "anon-before").await;
    let body = engine.inspect_container(&container).await.unwrap().unwrap();
    let source = |target: &str| {
        body.mounts
            .as_ref()
            .unwrap()
            .iter()
            .find(|mount| mount.destination.as_deref() == Some(target))
            .and_then(|mount| mount.name.clone())
            .unwrap_or_else(|| panic!("{target} 没有挂卷"))
    };
    let (named, anon) = (source("/named"), source("/anon"));
    assert_ne!(named, anon);

    // 撤权时 Matrix 不在：限制照样生效，原因留着。
    tuwunel.state().refuse_profile = true;
    let m = manager(engine.clone());
    m.restrict_bot(&id(), "alice", "撤权时 Matrix 不在".to_owned())
        .await
        .unwrap();
    assert!(!running(&engine, &container).await);
    let stored = || {
        super::super::store::Store::open(dir.path().join("plugins"))
            .unwrap()
            .bots()
            .load(&id())
            .unwrap()
    };
    assert_eq!(
        stored().bots["alice"].volumes[&container]["/anon"].as_str(),
        anon
    );

    // 核心重启：插件标异常、原因写明，Bot 的容器还是停着。
    let m = manager(engine.clone());
    m.restore().await.unwrap();
    let PluginStatus::Degraded { reason } = &m.list()[0].status else {
        panic!("待恢复的 Bot 没让插件异常：{:?}", m.list()[0].status);
    };
    assert!(
        reason.contains(&super::message(
            "plugin-install-resident-restricted",
            &[("bot", "alice"), ("reason", "撤权时 Matrix 不在")],
        )),
        "{reason}"
    );
    assert!(!running(&engine, &container).await);

    // 正常重试（资料还是写不进去也不挡恢复）：同一批卷，内容原样。
    m.add_bot(&id(), "alice", alice()).await.unwrap();
    assert!(running(&engine, &container).await);
    assert_eq!(stored().bots["alice"].restricted, None);
    assert_eq!(read(&engine, &container, "/named/f").await, "named-before");
    assert_eq!(read(&engine, &container, "/anon/f").await, "anon-before");
    let body = engine.inspect_container(&container).await.unwrap().unwrap();
    let after = |target: &str| {
        body.mounts
            .as_ref()
            .unwrap()
            .iter()
            .find(|mount| mount.destination.as_deref() == Some(target))
            .and_then(|mount| mount.name.clone())
    };
    assert_eq!(after("/named").as_deref(), Some(named.as_str()));
    assert_eq!(after("/anon").as_deref(), Some(anon.as_str()));

    m.uninstall(&id(), true, &super::langs()).await.unwrap();
    assert!(
        engine
            .inspect_container(&container)
            .await
            .unwrap()
            .is_none()
    );
}

fn alice() -> wonton_protocol::BotKeeper {
    wonton_protocol::BotKeeper::User {
        username: "alice".to_owned(),
    }
}
