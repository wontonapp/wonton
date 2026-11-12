//! 会话列表的行为测试：给定一串操作，断言核心给出的 ViewModel。
//!
//! 只经由导出给原生壳的接口（`ClientCore`、`Action`、订阅）驱动，两端共享这些结论。

use std::ops::Deref;
use std::sync::Arc;

use wonton_client::{Action, ClientCore, CoreError, Network, SessionListViewModel};

/// 测试用的核心，连同它的数据目录（存设备密钥），测试结束时一起删除。
struct TestCore {
    core: Arc<ClientCore>,
    _dir: tempfile::TempDir,
}

impl Deref for TestCore {
    type Target = ClientCore;

    fn deref(&self) -> &ClientCore {
        &self.core
    }
}

fn new_core() -> TestCore {
    let dir = tempfile::tempdir().unwrap();
    let core = ClientCore::with_network(dir.path().to_str().unwrap(), Network::DirectOnly).unwrap();
    TestCore { core, _dir: dir }
}

fn create(title: &str) -> Action {
    Action::CreateSession {
        title: title.into(),
    }
}

fn select(session_id: &str) -> Action {
    Action::SelectSession {
        session_id: session_id.into(),
    }
}

fn delete(session_id: &str) -> Action {
    Action::DeleteSession {
        session_id: session_id.into(),
    }
}

/// 依次执行操作，要求每一步都成功，返回最终的会话列表。
fn run(actions: impl IntoIterator<Item = Action>) -> (TestCore, SessionListViewModel) {
    let core = new_core();
    for action in actions {
        core.dispatch(action.clone())
            .unwrap_or_else(|error| panic!("{action:?} 失败：{error}"));
    }
    let view_model = core.session_list();
    (core, view_model)
}

/// 把 ViewModel 压成 (标题, 是否选中) 列表，方便断言。
fn rows(view_model: &SessionListViewModel) -> Vec<(&str, bool)> {
    view_model
        .sessions
        .iter()
        .map(|row| (row.title.as_str(), row.is_selected))
        .collect()
}

fn id_of<'a>(view_model: &'a SessionListViewModel, title: &str) -> &'a str {
    &view_model
        .sessions
        .iter()
        .find(|row| row.title == title)
        .unwrap_or_else(|| panic!("找不到会话 {title}"))
        .id
}

#[test]
fn a_new_core_has_no_sessions() {
    let (_, view_model) = run([]);
    assert!(view_model.sessions.is_empty());
}

#[test]
fn created_sessions_are_listed_newest_first_and_the_newest_is_selected() {
    let (_, view_model) = run([create("家里的 NAS"), create("周末旅行")]);
    assert_eq!(
        rows(&view_model),
        [("周末旅行", true), ("家里的 NAS", false)]
    );
}

#[test]
fn titles_are_trimmed() {
    let (_, view_model) = run([create("  相册整理 \n")]);
    assert_eq!(rows(&view_model), [("相册整理", true)]);
}

#[test]
fn selecting_a_session_moves_the_selection() {
    let (core, view_model) = run([create("家里的 NAS"), create("周末旅行")]);
    core.dispatch(select(id_of(&view_model, "家里的 NAS")))
        .unwrap();
    assert_eq!(
        rows(&core.session_list()),
        [("周末旅行", false), ("家里的 NAS", true)]
    );
}

#[test]
fn deleting_the_selected_session_clears_the_selection() {
    let (core, view_model) = run([create("家里的 NAS"), create("周末旅行")]);
    core.dispatch(delete(id_of(&view_model, "周末旅行")))
        .unwrap();
    assert_eq!(rows(&core.session_list()), [("家里的 NAS", false)]);
}

#[test]
fn deleting_another_session_keeps_the_selection() {
    let (core, view_model) = run([create("家里的 NAS"), create("周末旅行")]);
    core.dispatch(delete(id_of(&view_model, "家里的 NAS")))
        .unwrap();
    assert_eq!(rows(&core.session_list()), [("周末旅行", true)]);
}

#[test]
fn session_ids_stay_unique_after_deletion() {
    let (core, view_model) = run([create("一"), create("二")]);
    core.dispatch(delete(id_of(&view_model, "一"))).unwrap();
    core.dispatch(create("三")).unwrap();
    let view_model = core.session_list();
    assert_ne!(id_of(&view_model, "二"), id_of(&view_model, "三"));
}

#[test]
fn invalid_actions_are_rejected_and_leave_the_state_unchanged() {
    let (core, before) = run([create("家里的 NAS")]);
    for action in [create(" \t "), select("missing"), delete("missing")] {
        let result = core.dispatch(action.clone());
        assert!(
            matches!(result, Err(CoreError::InvalidInput { .. })),
            "{action:?} 应当被拒绝，实际 {result:?}"
        );
        assert_eq!(core.session_list(), before);
    }
}

mod subscription {
    use std::time::Duration;

    use tokio::time::timeout;

    use super::*;

    /// 等待订阅的下一个值；超时说明订阅没被唤醒，测试失败而不是挂住。
    async fn next(
        subscription: &wonton_client::SessionListSubscription,
    ) -> Option<SessionListViewModel> {
        timeout(Duration::from_secs(5), subscription.next())
            .await
            .expect("订阅没有在 5 秒内返回")
    }

    #[tokio::test]
    async fn first_value_is_the_current_view_model() {
        let (core, view_model) = run([create("家里的 NAS")]);
        let subscription = core.subscribe_session_list();
        assert_eq!(next(&subscription).await, Some(view_model));
    }

    #[tokio::test]
    async fn every_dispatch_reaches_a_waiting_subscriber() {
        let core = new_core();
        let subscription = core.subscribe_session_list();
        assert_eq!(
            next(&subscription).await,
            Some(SessionListViewModel::default())
        );

        let waiting = tokio::spawn({
            let subscription = Arc::clone(&subscription);
            async move { subscription.next().await }
        });
        tokio::task::yield_now().await;
        core.dispatch(create("家里的 NAS")).unwrap();

        let view_model = waiting.await.unwrap().expect("订阅提前结束");
        assert_eq!(rows(&view_model), [("家里的 NAS", true)]);
    }

    #[tokio::test]
    async fn a_slow_subscriber_only_sees_the_latest_state() {
        let core = new_core();
        let subscription = core.subscribe_session_list();
        next(&subscription).await;

        core.dispatch(create("一")).unwrap();
        core.dispatch(create("二")).unwrap();

        let view_model = next(&subscription).await.unwrap();
        assert_eq!(rows(&view_model), [("二", true), ("一", false)]);
    }

    #[tokio::test]
    async fn rejected_actions_do_not_wake_subscribers() {
        let (core, _) = run([create("家里的 NAS")]);
        let subscription = core.subscribe_session_list();
        next(&subscription).await;

        core.dispatch(select("missing")).unwrap_err();
        let woke = timeout(Duration::from_millis(50), subscription.next()).await;
        assert!(woke.is_err(), "被拒绝的操作不应产生新的 ViewModel");
    }

    #[tokio::test]
    async fn cancel_ends_a_pending_and_all_later_waits() {
        let core = new_core();
        let subscription = core.subscribe_session_list();
        next(&subscription).await;

        let waiting = tokio::spawn({
            let subscription = Arc::clone(&subscription);
            async move { subscription.next().await }
        });
        tokio::task::yield_now().await;
        subscription.cancel();

        assert_eq!(waiting.await.unwrap(), None);
        core.dispatch(create("家里的 NAS")).unwrap();
        assert_eq!(next(&subscription).await, None);
    }

    #[tokio::test]
    async fn dropping_the_core_ends_the_subscription() {
        let core = new_core();
        let subscription = core.subscribe_session_list();
        next(&subscription).await;

        drop(core);
        assert_eq!(next(&subscription).await, None);
    }
}
