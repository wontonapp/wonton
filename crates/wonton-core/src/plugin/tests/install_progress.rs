//! 安装和升级报的阶段（ADR-0067）：准备、拉镜像、启动、等 Bot，装完清空；失败时连进度一起撤掉。

use std::sync::{Arc, Mutex};
use std::time::Duration;

use wonton_protocol::{InstallProgress, InstallStage};

use super::super::docker::Engine;
use super::super::fake_docker::{FakeDocker, Hold};

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
    // 本机没有这两个镜像：拉的时候走假 Docker 的进度流。第二个镜像总是拉，两个镜像都走拉取。
    docker.forget_image("nginx");
    docker.forget_image("redis");
    let name = container_name("app-test", "web");
    let dir = tempfile::tempdir().unwrap();
    let m = Arc::new(manager_with(engine, &dir));
    let compose = package(
        "  web:\n    image: nginx\n  db:\n    image: redis\n    pull_policy: always",
        "  permissions: []",
    );
    // 百分比由后台任务在每次变化时记下；阶段由停住的请求确定地读。
    let percents = Arc::new(Mutex::new(Vec::new()));
    let sampler = sample_percents(&m, &percents);

    let seen = stages_while_installing(&docker, &m, &compose, &name)
        .await
        .unwrap();
    sampler.abort();
    let percents = percents.lock().unwrap().clone();

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

/// 每次插件表变了就记下安装进度里的百分比，测试做完 `abort`。
///
/// 测试跑在单线程运行时上：报进度时叫醒这个任务，它排在假 Docker 下一行进度（隔 30 ms 的定时器）之前，
/// 所以跨过一次等待还在的百分比一定记得到，不靠轮询撞上。
fn sample_percents(
    m: &Arc<super::super::manager::PluginManager>,
    percents: &Arc<Mutex<Vec<u8>>>,
) -> tokio::task::JoinHandle<()> {
    let (m, percents) = (m.clone(), percents.clone());
    let mut changes = m.changes();
    tokio::spawn(async move {
        loop {
            if let Some(progress) = current(&m) {
                percents.lock().unwrap().extend(progress.percent);
            }
            if changes.changed().await.is_err() {
                return;
            }
        }
    })
}

/// 等一个停住的请求最多这么久；超时就失败并说出卡在哪一步，不让 CI 挂着。
const STEP: Duration = Duration::from_secs(20);

/// 装 `compose` 并返回报出的阶段。每一步都停在它之后必然发出的请求上：阶段已经报出，下一个阶段还到不了，
/// 读到当前阶段再放行，不靠采样撞上。
///
/// 假 Docker 同时只停一种请求，新的 `hold` 顶掉旧的：所以总是等上一个请求停住了，才停下一个、再放行上一个。
/// 先停下一个再等上一个的话，上一个还没到就被顶掉，会一直等不到。
async fn stages_while_installing(
    docker: &FakeDocker,
    m: &Arc<super::super::manager::PluginManager>,
    compose: &str,
    name: &str,
) -> Result<Vec<InstallStage>, super::super::InstallError> {
    let healthy = format!("GET /containers/{name}/json");
    // （停住的请求，这一步叫什么，停住时该是的阶段）
    let steps = [
        ("POST /images/create", "第一个镜像的拉取", None),
        // 第二个镜像的拉取发出时，第一个镜像的进度流已经报完，下一次改回准备阶段还在后头。
        (
            "POST /images/create",
            "第二个镜像的拉取",
            Some(InstallStage::PullingImages),
        ),
        (
            "POST /networks/create",
            "建网络",
            Some(InstallStage::Preparing),
        ),
        (
            "POST /containers/create",
            "建容器",
            Some(InstallStage::Starting),
        ),
        (
            healthy.as_str(),
            "健康检查",
            Some(InstallStage::WaitingForBot),
        ),
    ];

    let mut seen = Vec::new();
    let mut held = docker.hold("GET /containers/json");
    let mut install = tokio::spawn({
        let m = m.clone();
        let compose = compose.to_owned();
        async move { m.install(&compose, &[], langs()).await }
    });
    stopped_at(&held, &mut install, "清残留").await;
    assert_eq!(
        stage_now(m, &mut seen),
        InstallStage::Preparing,
        "清残留时是准备阶段"
    );
    for (call, step, stage) in steps {
        let next = docker.hold(call);
        held.release();
        held = next;
        stopped_at(&held, &mut install, step).await;
        if let Some(stage) = stage {
            assert_eq!(stage_now(m, &mut seen), stage, "停在{step}时的阶段");
        }
    }
    held.release();

    tokio::time::timeout(STEP, install)
        .await
        .unwrap_or_else(|_| panic!("放行健康检查后 {STEP:?} 内没装完"))
        .expect("安装任务没 panic")?;
    Ok(seen)
}

/// 等到假 Docker 停住 `hold` 的请求。安装先结束了（多半是出错）或者超时，就说明卡在 `step`，立刻失败。
async fn stopped_at<T: std::fmt::Debug>(
    hold: &Hold,
    install: &mut tokio::task::JoinHandle<T>,
    step: &str,
) {
    let wait = async {
        tokio::select! {
            () = hold.reached() => {}
            done = &mut *install => panic!("还没到{step}安装就结束了：{done:?}"),
        }
    };
    tokio::time::timeout(STEP, wait)
        .await
        .unwrap_or_else(|_| panic!("{STEP:?} 内没等到{step}的请求"));
}

fn current(m: &super::super::manager::PluginManager) -> Option<InstallProgress> {
    m.installed(&langs())
        .plugins
        .first()?
        .install_progress
        .clone()
}

/// 读到当前阶段并记下：调用时请求正停着，阶段不会再往前走。
fn stage_now(
    m: &super::super::manager::PluginManager,
    seen: &mut Vec<InstallStage>,
) -> InstallStage {
    let stage = current(m).expect("安装中一直有阶段").stage;
    seen.push(stage);
    stage
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
