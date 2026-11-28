//! 安装结果从服务器的已安装列表等：请求断了、列表读取被按住都不算失败，放行后照常落地（ADR-0017）。

use std::time::{Duration, Instant};

use axum::http::Method;
use wonton_client::{Action, PluginState, PluginTaskState};

use super::rig::{CONNECT_BACKSTOP, Canned, Rig, installed, preview, running};

/// 兜底预算，不是节奏：按生产常数推导——三次成功读取最坏 3 × (3s 间隔 + 30s 读取限时) = 99s，
/// 之后 settle 自己在最后一次成功读取 120s 后转不确定（UNREACHABLE_LIMIT），合计 219s；取 240s。
/// 预算不足时超时会报出当时状态。
const READ_BACKSTOP: Duration = Duration::from_secs(240);

/// 家里的服务器单独执行安装：请求断了不算失败，结果从已安装列表里看（ADR-0017）。
#[tokio::test]
async fn a_dropped_connection_waits_for_the_result_on_the_server() {
    let rig = Rig::new().await.without_periodic_checks();
    rig.connected_within(CONNECT_BACKSTOP).await;
    rig.wait_for(|view| view.installed.is_some()).await;

    rig.core.set_lists(vec![
        vec![installed("app.photos", PluginState::Installing)],
        vec![running("app.photos")],
    ]);
    // 本地代理连不上隧道时回没有错误体的 502。
    rig.core.answer(
        Method::POST,
        "/_wonton/api/plugins",
        Canned(502, String::new()),
    );
    rig.app
        .dispatch(Action::Plugins {
            action: wonton_protocol::PluginAction::InstallPlugin {
                preview: preview("app.photos"),
                secrets: Default::default(),
                settings: Default::default(),
            },
        })
        .unwrap();
    let state = rig
        .task_within("app.photos", READ_BACKSTOP, |state| {
            *state != PluginTaskState::Running
        })
        .await;
    assert_eq!(state, PluginTaskState::Succeeded);

    rig.core.set_lists(vec![vec![]]);
    rig.core.answer(
        Method::POST,
        "/_wonton/api/plugins",
        Canned(502, String::new()),
    );
    rig.app
        .dispatch(Action::Plugins {
            action: wonton_protocol::PluginAction::InstallPlugin {
                preview: preview("app.other"),
                secrets: Default::default(),
                settings: Default::default(),
            },
        })
        .unwrap();
    let state = rig
        .task_within("app.other", READ_BACKSTOP, |state| {
            *state != PluginTaskState::Running
        })
        .await;
    assert!(
        matches!(state, PluginTaskState::Uncertain { .. }),
        "服务器上一直没有它，如实说不确定：{state:?}"
    );
}

/// 列表读取被按住（繁忙时请求会这样停滞）：停滞不算失败，放行后结果照常从已安装列表落地。
#[tokio::test]
async fn a_stalled_list_read_does_not_fail_the_install() {
    let rig = Rig::new().await.without_periodic_checks();
    rig.connected_within(CONNECT_BACKSTOP).await;
    rig.wait_for(|view| view.installed.is_some()).await;

    rig.core.set_lists(vec![vec![running("app.photos")]]);
    rig.core.hold(Method::GET, "/_wonton/api/plugins");
    rig.core.answer(
        Method::POST,
        "/_wonton/api/plugins",
        Canned(502, String::new()),
    );
    rig.app
        .dispatch(Action::Plugins {
            action: wonton_protocol::PluginAction::InstallPlugin {
                preview: preview("app.photos"),
                secrets: Default::default(),
                settings: Default::default(),
            },
        })
        .unwrap();

    // 先等读取到达核心并被按住；这期间任务如实停在 Running。
    let arrived = rig.core.seen(&Method::GET, "/_wonton/api/plugins").len();
    let deadline = Instant::now() + READ_BACKSTOP;
    while rig.core.seen(&Method::GET, "/_wonton/api/plugins").len() == arrived {
        assert!(Instant::now() < deadline, "列表读取一直没到核心");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(rig.app.plugins().tasks[0].state, PluginTaskState::Running);

    rig.core.release(Method::GET, "/_wonton/api/plugins");
    let state = rig
        .task_within("app.photos", READ_BACKSTOP, |state| {
            *state != PluginTaskState::Running
        })
        .await;
    assert_eq!(state, PluginTaskState::Succeeded);
}
