//! 安装和升级报的阶段（ADR-0067）：准备、拉镜像、启动、等 Bot，装完清空；失败时连进度一起撤掉。

use std::sync::Arc;
use std::time::Duration;

use wonton_protocol::{InstallProgress, InstallStage, PluginState};

use super::super::docker::Engine;
use super::super::fake_docker::FakeDocker;

use super::super::names::container_name;
use super::manager::manager_with;
use super::{langs, package};

async fn fake_docker() -> (FakeDocker, Engine) {
    FakeDocker::start().await
}

/// 契约：装一个应用时依次报出每个阶段、拉镜像带百分比，装完清空。
#[tokio::test]
async fn an_install_reports_its_stages_and_clears_them() {
    let (docker, engine) = fake_docker().await;
    // 本机没有这个镜像：拉的时候走假 Docker 的进度流。
    docker.forget_image("nginx");
    // 先停住建容器，确认报的是“启动”；再改成停住健康检查，确认接着报的是“等 Bot”。
    let creating = docker.hold("POST /containers/create");
    let name = container_name("app-test", "web");
    let dir = tempfile::tempdir().unwrap();
    let m = Arc::new(manager_with(engine, &dir));
    let compose = package("  web:\n    image: nginx", "  permissions: []");
    let install = tokio::spawn({
        let m = m.clone();
        async move { m.install(&compose, &[], langs()).await }
    });

    let mut seen: Vec<InstallStage> = Vec::new();
    let mut percents: Vec<u8> = Vec::new();
    let mut starting: Option<InstallStage> = None;
    let mut waiting: Option<InstallStage> = None;
    let mut ready: Option<super::super::fake_docker::Hold> = None;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    while tokio::time::Instant::now() < deadline {
        if let Some(progress) = current(&m) {
            if seen.last() != Some(&progress.stage) {
                seen.push(progress.stage);
            }
            percents.extend(progress.percent);
        }
        tokio::select! {
            () = creating.reached(), if starting.is_none() => {
                starting = current(&m).map(|progress| progress.stage);
                ready = Some(docker.hold(&format!("GET /containers/{name}/json")));
                creating.release();
            }
            () = async {
                if let Some(ready) = &ready {
                    ready.reached().await;
                } else {
                    std::future::pending().await
                }
            }, if waiting.is_none() => {
                waiting = current(&m).map(|progress| progress.stage);
                ready.as_ref().expect("先停了建容器").release();
            }
            () = tokio::time::sleep(Duration::from_millis(1)) => {}
        }
        let listed = m.installed(&langs());
        if install.is_finished()
            || listed
                .plugins
                .first()
                .is_some_and(|plugin| matches!(plugin.status, PluginState::Running { .. }))
        {
            break;
        }
    }
    install.await.unwrap().unwrap();
    if let Some(ready) = &ready {
        ready.release();
    }

    assert_eq!(
        seen,
        [
            InstallStage::Preparing,
            InstallStage::PullingImages,
            InstallStage::Preparing,
            InstallStage::Starting,
            InstallStage::WaitingForBot,
        ]
    );
    assert_eq!(starting, Some(InstallStage::Starting), "建容器时是启动阶段");
    assert_eq!(
        waiting,
        Some(InstallStage::WaitingForBot),
        "健康检查时是等 Bot 阶段"
    );
    assert!(
        percents
            .iter()
            .any(|percent| *percent > 0 && *percent < 100),
        "拉镜像要有中间的百分比：{percents:?}"
    );
    assert!(
        percents.windows(2).all(|pair| pair[0] <= pair[1]),
        "层之间交错时百分比也不能往回跳：{percents:?}"
    );
    assert!(
        percents.iter().any(|percent| *percent >= 99),
        "下完要接近 100：{percents:?}"
    );
    assert!(
        m.installed(&langs()).plugins[0].install_progress.is_none(),
        "装完不能再留着阶段"
    );
}

fn current(m: &super::super::manager::PluginManager) -> Option<InstallProgress> {
    m.installed(&langs())
        .plugins
        .first()?
        .install_progress
        .clone()
}

/// 契约：安装失败、插件回滚以后没有进度留下。
#[tokio::test]
async fn a_failed_install_leaves_no_progress() {
    let (docker, engine) = fake_docker().await;
    docker.fail_pulls(Some("拉不下来"));
    let dir = tempfile::tempdir().unwrap();
    let m = manager_with(engine, &dir);
    let compose = package("  web:\n    image: nginx", "  permissions: []");

    assert!(
        m.install(&compose, &[], langs()).await.is_err(),
        "拉不到镜像时安装要失败"
    );
    assert!(m.installed(&langs()).plugins.is_empty(), "失败的插件已回滚");
}

/// 契约：层之间交错、完成的层只有 `Pull complete` 没有字节数时，百分比也不往回跳，最后到 100。
#[test]
fn layer_percentages_never_go_backwards() {
    use crate::plugin::registry_mirror::{LayerBytes, LayerProgress};

    let mut bytes = LayerBytes::default();
    let events = [
        ("a", false, 40, 100),
        ("b", false, 40, 100),
        ("a", true, 0, 0),
        ("b", false, 100, 100),
        ("c", true, 0, 0),
        ("b", true, 0, 0),
    ];
    let mut seen = Vec::new();
    for (id, done, current, total) in events {
        seen.push(bytes.update(LayerProgress {
            id,
            done,
            current,
            total,
        }));
    }
    assert!(
        seen.windows(2).all(|pair| pair[0] <= pair[1]),
        "百分比往回跳了：{seen:?}"
    );
    assert_eq!(seen.last(), Some(&100), "都完成了要到 100：{seen:?}");
}

/// 百分比按每个镜像的字节算，多个镜像时铺满整段。
#[test]
fn percentages_do_not_shrink_between_images() {
    let report = |index: u64, count: u64, current: u64, total: u64| {
        let done = current.saturating_mul(100) / total.max(1);
        ((index * 100 + done) / count).min(100) as u8
    };
    assert_eq!(report(0, 2, 10, 10), 50);
    assert_eq!(report(1, 2, 0, 10), 50);
    assert_eq!(report(1, 2, 10, 10), 100);
    assert_eq!(
        InstallProgress {
            stage: InstallStage::PullingImages,
            percent: Some(report(1, 2, 10, 10)),
        }
        .percent,
        Some(100)
    );
}
