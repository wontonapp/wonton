//! 出错页的“重试”：进行中、不能连点、结果如实写回这一份实例（ADR-0026 的“动作按钮的反馈”）。

use super::*;
use wonton_protocol::{ActionFailure, ActionRun, PluginState, TabState};

fn stack_page() -> PlacedTab {
    PlacedTab::Page { tab: page_tab() }
}

/// 重试在后台任务里跑，任务靠弱引用找回 PagesLink：测试的 Rig 默认不设，这里补上。
fn rig(external: bool) -> Rig {
    let rig = if external {
        Rig::for_account(true)
    } else {
        Rig::new()
    };
    *lock(&rig.tabs.weak) = Arc::downgrade(&rig.tabs);
    rig
}

fn set_status(rig: &Rig, status: PluginState) {
    rig.plugins.send_modify(|view| {
        view.installed.as_mut().unwrap().plugins[0].status = status;
    });
    rig.tabs.recompute();
}

async fn settled(rig: &Rig) -> ActionRun {
    for _ in 0..200 {
        let run = rig.page(&page_tab()).retry;
        if !run.is_running() {
            return run;
        }
        tokio::task::yield_now().await;
    }
    rig.page(&page_tab()).retry
}

async fn reloaded_past(rig: &Rig, before: u64) {
    for _ in 0..200 {
        if rig.page(&page_tab()).reload > before {
            return;
        }
        tokio::task::yield_now().await;
    }
    panic!("the page was never reloaded");
}

#[tokio::test]
async fn retrying_a_plugin_that_is_still_broken_says_so_and_ignores_repeat_taps() {
    let rig = rig(true);
    rig.add_app(plugin_app("app.shared"));
    set_status(
        &rig,
        PluginState::Degraded {
            reason: "container exited".into(),
        },
    );
    rig.open(page_tab());
    assert!(matches!(
        rig.page(&page_tab()).state,
        TabState::Failed { .. }
    ));

    rig.tabs
        .dispatch(PageAction::RetryPage { page: stack_page() })
        .unwrap();
    assert_eq!(rig.page(&page_tab()).retry, ActionRun::Running);
    let reload = rig.page(&page_tab()).reload;
    // 正在重试时再点：安静地什么也不做，不再起一次。
    rig.tabs
        .dispatch(PageAction::RetryPage { page: stack_page() })
        .unwrap();
    assert_eq!(rig.page(&page_tab()).retry, ActionRun::Running);

    assert_eq!(
        settled(&rig).await,
        ActionRun::Failed {
            failure: ActionFailure::StillFailing,
            details: None
        }
    );
    assert_eq!(rig.page(&page_tab()).reload, reload);
}

#[tokio::test]
async fn retrying_a_running_plugin_reloads_and_ends_when_the_page_loads() {
    let rig = rig(true);
    rig.add_app(plugin_app("app.shared"));
    rig.open(page_tab());
    rig.tabs
        .dispatch(PageAction::TabLoadFailed {
            page: stack_page(),
            status: Some(502),
        })
        .unwrap();
    let before = rig.page(&page_tab()).reload;

    rig.tabs
        .dispatch(PageAction::RetryPage { page: stack_page() })
        .unwrap();
    reloaded_past(&rig, before).await;
    // 网页还没报结果：仍在进行中。
    assert_eq!(rig.page(&page_tab()).retry, ActionRun::Running);
    assert_eq!(rig.page(&page_tab()).state, TabState::Ready);

    rig.tabs
        .dispatch(PageAction::TabLoadSucceeded { page: stack_page() })
        .unwrap();
    assert_eq!(rig.page(&page_tab()).retry, ActionRun::Idle);
}

#[tokio::test]
async fn a_retry_that_loads_badly_again_is_still_failing() {
    let rig = rig(true);
    rig.add_app(plugin_app("app.shared"));
    rig.open(page_tab());
    let before = rig.page(&page_tab()).reload;
    rig.tabs
        .dispatch(PageAction::RetryPage { page: stack_page() })
        .unwrap();
    reloaded_past(&rig, before).await;

    rig.tabs
        .dispatch(PageAction::TabLoadFailed {
            page: stack_page(),
            status: Some(502),
        })
        .unwrap();
    assert!(matches!(
        rig.page(&page_tab()).retry,
        ActionRun::Failed {
            failure: ActionFailure::StillFailing,
            details: None
        }
    ));

    // 后来页面自己加载好了：上一次没成的那句不再挂着。
    rig.tabs
        .dispatch(PageAction::TabLoadSucceeded { page: stack_page() })
        .unwrap();
    assert_eq!(rig.page(&page_tab()).retry, ActionRun::Idle);
}

#[tokio::test]
async fn retrying_without_a_connection_is_a_transport_failure() {
    let rig = rig(false);
    rig.add_app(plugin_app("app.shared"));
    rig.open(page_tab());
    rig.tabs
        .dispatch(PageAction::RetryPage { page: stack_page() })
        .unwrap();
    assert!(matches!(
        settled(&rig).await,
        ActionRun::Failed {
            failure: ActionFailure::Transport,
            details: Some(_)
        }
    ));
}

#[tokio::test(start_paused = true)]
async fn a_retry_whose_page_never_reports_becomes_uncertain() {
    let rig = rig(true);
    rig.add_app(plugin_app("app.shared"));
    rig.open(page_tab());
    let before = rig.page(&page_tab()).reload;
    rig.tabs
        .dispatch(PageAction::RetryPage { page: stack_page() })
        .unwrap();
    reloaded_past(&rig, before).await;

    tokio::time::sleep(super::super::retry::RETRY_TIMEOUT * 2).await;
    assert_eq!(rig.page(&page_tab()).retry, ActionRun::Uncertain);
}

#[tokio::test]
async fn retries_are_kept_per_instance() {
    let rig = rig(true);
    rig.add_app(plugin_app("app.shared"));
    rig.open(page_tab());
    rig.tabs
        .dispatch(PageAction::AddPanelPage {
            scope: main_scope(),
            tab: page_tab(),
        })
        .unwrap();
    rig.tabs
        .dispatch(PageAction::RetryPage {
            page: PlacedTab::Panel {
                scope: main_scope(),
                tab: page_tab(),
            },
        })
        .unwrap();
    assert_eq!(rig.view().panel[0].page.retry, ActionRun::Running);
    assert_eq!(rig.page(&page_tab()).retry, ActionRun::Idle);
}
