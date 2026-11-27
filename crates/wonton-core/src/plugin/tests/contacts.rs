//! 联系人名单的应用回执（#646）：在假 Docker 上验证运行中的插件真的换上了名单，停着的、Docker 不可达的、
//! 起不来的都按原因如实报，不把“任务已经起来”当已应用。

use std::sync::Arc;

use serde_json::json;
use wonton_protocol::{BotRef, ContactApply, ContactApplyFailure};

use super::super::PluginStatus;
use super::super::docker::Engine;
use super::super::fake_docker::FakeDocker;
use super::super::manager::PluginManager;
use super::super::manifest::PluginId;
use super::super::package::PluginPackage;
use super::manager::manager_in_container;
use super::package;

/// 一个回声 Bot：服务里带上名单变量。
fn echo() -> String {
    package(
        "  bot:\n    image: echo\n    environment:\n      \
         ALLOWED: ${WONTON_MATRIX_ALLOWED_USERS}",
        "  bot: {display_name: {zh: 回声, en: Echo}}\n  permissions: [matrix.bot]",
    )
    .replace("id: app.test", "id: app.echo")
}

fn granted() -> Vec<super::super::Permission> {
    vec!["matrix.bot".parse().unwrap()]
}

fn bot() -> BotRef {
    BotRef {
        plugin_id: "app.echo".into(),
        bot: "default".into(),
    }
}

/// 单 Bot 插件的服务名就是容器名，不带 `--default`。
fn container() -> String {
    super::super::names::container_name("app-echo", "bot")
}

struct Bed {
    docker: FakeDocker,
    matrix: Arc<crate::matrix::Matrix>,
    _tuwunel: crate::matrix::fake::Fake,
    manager: Arc<PluginManager>,
    dir: tempfile::TempDir,
}

impl Bed {
    /// 装好一个运行中的回声 Bot，名单里已经有一个联系人。
    async fn installed() -> Self {
        let (docker, engine) = FakeDocker::start().await;
        let (tuwunel, config) = crate::matrix::fake::start().await;
        let dir = tempfile::tempdir().unwrap();
        let matrix = crate::matrix::Matrix::connect(
            &config,
            &dir.path().join("core"),
            crate::http::HttpClient::new(),
        )
        .await
        .unwrap();
        let matrix = Arc::new(matrix);
        let manager =
            Arc::new(manager_in_container(engine, &dir).with_matrix(Some(matrix.clone())));
        manager
            .install(&echo(), &granted(), super::langs())
            .await
            .unwrap();
        manager.set_contact_source(Arc::new(|user_id: &str| {
            if user_id == "@echo:test" {
                vec!["@alice:test".to_owned()]
            } else {
                Vec::new()
            }
        }));
        Self {
            docker,
            matrix,
            _tuwunel: tuwunel,
            manager,
            dir,
        }
    }

    fn allowed(&self) -> Option<String> {
        self.docker
            .env(&container())
            .into_iter()
            .find_map(|entry| entry.strip_prefix("ALLOWED=").map(str::to_owned))
    }
}

#[tokio::test]
async fn a_running_plugin_gets_the_new_list_and_reports_applied() {
    let bed = Bed::installed().await;
    assert_eq!(
        bed.allowed().as_deref(),
        Some("@owner:test"),
        "装好时只有归属人：{:?} 容器 {:?}",
        bed.docker.env(&container()),
        bed.docker.containers()
    );

    let apply = bed.manager.rebuild_for_contacts(&bot()).await;
    assert_eq!(apply, ContactApply::Applied);
    assert_eq!(
        bed.allowed().as_deref(),
        Some("@owner:test,@alice:test"),
        "新名单在换上的容器里"
    );
}

#[tokio::test]
async fn a_stopped_plugin_reports_not_ready() {
    let bed = Bed::installed().await;
    bed.manager
        .stop_detached(PluginId::parse("app.echo").unwrap())
        .await
        .unwrap();
    let apply = bed.manager.rebuild_for_contacts(&bot()).await;
    let ContactApply::Failed { reason, detail } = apply else {
        panic!("停着的插件不能报 {apply:?}");
    };
    assert_eq!(reason, ContactApplyFailure::PluginNotReady);
    assert!(detail.contains("没在运行"), "{detail}");
}

#[tokio::test]
async fn an_unreachable_docker_reports_transport_unavailable() {
    let bed = Bed::installed().await;
    // 同一份记录换一个连不上的 Docker：重建在碰容器之前就失败。
    let other = manager_in_container(Engine::unreachable_for_test(), &bed.dir)
        .with_matrix(Some(bed.matrix.clone()));
    let manifest = PluginPackage::parse(&echo(), &super::langs())
        .unwrap()
        .manifest
        .clone();
    other.register_for_test(manifest, PluginStatus::Running, None);
    let apply = other.rebuild_for_contacts(&bot()).await;
    let (reason, detail) = match apply {
        ContactApply::Failed { reason, detail } => (reason, detail),
        ContactApply::Applied | ContactApply::Other => panic!("Docker 连不上不能报成功"),
    };
    assert_eq!(
        reason,
        ContactApplyFailure::TransportUnavailable,
        "{detail}"
    );
}

#[tokio::test]
async fn a_service_that_does_not_come_up_reports_upstream() {
    let bed = Bed::installed().await;
    bed.docker.set_start_state(
        &container(),
        json!({"Status": "exited", "Running": false, "ExitCode": 1}),
    );
    let apply = bed.manager.rebuild_for_contacts(&bot()).await;
    let (reason, detail) = match apply {
        ContactApply::Failed { reason, detail } => (reason, detail),
        ContactApply::Applied | ContactApply::Other => panic!("起不来的服务不能报成功"),
    };
    assert_eq!(reason, ContactApplyFailure::Upstream, "{detail}");
    assert!(!detail.is_empty());
}

#[tokio::test]
async fn an_invalid_plugin_id_reports_invalid_input() {
    let bed = Bed::installed().await;
    let apply = bed
        .manager
        .rebuild_for_contacts(&BotRef {
            plugin_id: "没有冒号".into(),
            bot: "default".into(),
        })
        .await;
    assert_eq!(
        match apply {
            ContactApply::Failed { reason, .. } => reason,
            ContactApply::Applied | ContactApply::Other => panic!("插件 ID 不合法不能报成功"),
        },
        ContactApplyFailure::InvalidInput
    );
}
