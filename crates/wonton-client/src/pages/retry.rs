//! 出错页上的“重试”（ADR-0026 的“动作按钮的反馈”）：按实例记进度，结果如实写回 [`TabPage::retry`]。
//!
//! 先重新取一次插件状态：插件还是坏的，结果就是“还是不行”；好了就重新加载这一份，等壳报网页加载成没成。
//! 壳一直不报（WebView 卡住、页面交给了系统浏览器）时过 [`RETRY_TIMEOUT`] 改成不确定，按钮不会一直转。
//!
//! [`TabPage::retry`]: wonton_protocol::TabPage::retry

use std::time::Duration;

use wonton_protocol::{ActionFailure, ActionRun, PluginFailure, PluginProblem};

use super::*;

/// 重新加载以后等壳报结果的上限。插件页经隧道首次加载，慢的十几秒（Immich 实测）。
pub(super) const RETRY_TIMEOUT: Duration = Duration::from_secs(30);

/// 一份实例的重试：`id` 区分先后两次，前一次的超时不改后一次。
#[derive(Clone, Debug)]
pub(super) struct PageRetry {
    pub(super) id: u64,
    pub(super) run: ActionRun,
}

impl PagesLink {
    pub(super) fn retry(&self, page: PlacedTab) -> Result<(), CoreError> {
        let page = canonical_page(page);
        let mut started = None;
        self.runtime_change(|state, _| {
            if !is_web_page(placed_tab_ref(&page)) || !state_has_page(state, &page) {
                return Err(invalid(&l10n!(
                    CATALOG,
                    language(),
                    "tabs-page-cannot-reload"
                )));
            }
            if state
                .retries
                .get(&page)
                .is_some_and(|retry| retry.run.is_running())
            {
                return Ok(());
            }
            state.retry_seq = state.retry_seq.wrapping_add(1);
            let id = state.retry_seq;
            state.retries.insert(
                page.clone(),
                PageRetry {
                    id,
                    run: ActionRun::Running,
                },
            );
            started = Some(id);
            Ok(())
        })?;
        let Some(id) = started else { return Ok(()) };
        let weak = lock(&self.weak).clone();
        self.runtime.spawn(async move {
            let Some(link) = weak.upgrade() else { return };
            let refreshed = link.plugins.refresh_installed_now().await;
            link.retry_refreshed(&page, id, refreshed);
            drop(link);
            tokio::time::sleep(RETRY_TIMEOUT).await;
            if let Some(link) = weak.upgrade() {
                link.settle_retry(&page, id, ActionRun::Uncertain);
            }
        });
        Ok(())
    }

    /// 插件状态取回来了：插件还是坏的就收尾，好了就重新加载这一份，等网页的结果。
    fn retry_refreshed(&self, page: &PlacedTab, id: u64, refreshed: Result<(), PluginProblem>) {
        let _ = self.runtime_change(|state, plugins| {
            if !state
                .retries
                .get(page)
                .is_some_and(|retry| retry.id == id && retry.run.is_running())
            {
                return Ok(());
            }
            let outcome = match refreshed {
                Err(problem) => Some(ActionRun::Failed {
                    failure: refresh_failure(problem.reason),
                    details: Some(problem.message),
                }),
                Ok(()) => plugin_outcome(placed_tab_ref(page), plugins),
            };
            match outcome {
                Some(run) => set_run(state, page, run),
                None => {
                    state.failures.remove(page);
                    increment_reload(state, page);
                }
            }
            Ok(())
        });
    }

    /// 这一次还在跑才改：后来又点了一次、页面已经关了，都不动。
    fn settle_retry(&self, page: &PlacedTab, id: u64, run: ActionRun) {
        let _ = self.runtime_change(|state, _| {
            if state
                .retries
                .get(page)
                .is_some_and(|retry| retry.id == id && retry.run.is_running())
            {
                set_run(state, page, run);
            }
            Ok(())
        });
    }
}

/// 壳报了这一份网页的加载结果：在重试的话，成了就结束（页面自己就是结果），没成就是“还是不行”。
/// 加载成功也清掉上一次没成的结果，免得页面以后再出错时还挂着旧的那句。没成的原因和原文出错页上已经写着
/// （`TabState::LoadFailed`），结果不再带一份。
pub(super) fn retry_loaded(state: &mut PagesState, page: &PlacedTab, loaded: bool) {
    let Some(retry) = state.retries.get(page) else {
        return;
    };
    if loaded {
        state.retries.remove(page);
    } else if retry.run.is_running() {
        set_run(
            state,
            page,
            ActionRun::Failed {
                failure: ActionFailure::StillFailing,
                details: None,
            },
        );
    }
}

/// 已经不在的实例不留重试；换服务器时全清。
pub(super) fn drop_gone_retries(state: &mut PagesState) {
    let gone: Vec<PlacedTab> = state
        .retries
        .keys()
        .filter(|page| !state_has_page(state, page))
        .cloned()
        .collect();
    for page in gone {
        state.retries.remove(&page);
    }
}

fn set_run(state: &mut PagesState, page: &PlacedTab, run: ActionRun) {
    match run {
        ActionRun::Idle => {
            state.retries.remove(page);
        }
        run => {
            if let Some(retry) = state.retries.get_mut(page) {
                retry.run = run;
            }
        }
    }
}

/// 插件状态取回来以后这一份怎么收尾；`None` 是插件在跑，要重新加载网页再看。
fn plugin_outcome(tab: &TabRef, plugins: &PluginsViewModel) -> Option<ActionRun> {
    let plugin_id = tab_ref_plugin_id(tab)?;
    let plugin = plugins
        .installed
        .as_ref()
        .and_then(|installed| installed.plugins.iter().find(|p| p.id == plugin_id));
    let Some(plugin) = plugin else {
        return Some(ActionRun::Failed {
            failure: ActionFailure::NotReady,
            details: None,
        });
    };
    match &plugin.status {
        PluginState::Running { .. } | PluginState::NeedsConfiguration => None,
        // 起着、升级着：页面自己显示进度，重试到这里就算交出去了。
        PluginState::Installing | PluginState::Upgrading => Some(ActionRun::Idle),
        // 原因页面上已经写着，这里不再重复一遍。
        PluginState::Degraded { .. } | PluginState::Stopped | PluginState::Unknown => {
            Some(ActionRun::Failed {
                failure: ActionFailure::StillFailing,
                details: None,
            })
        }
        PluginState::Uninstalling => Some(ActionRun::Failed {
            failure: ActionFailure::NotReady,
            details: None,
        }),
    }
}

fn refresh_failure(reason: PluginFailure) -> ActionFailure {
    match reason {
        PluginFailure::TransportUnavailable => ActionFailure::Transport,
        PluginFailure::Unauthorized => ActionFailure::Permission,
        PluginFailure::NotFound => ActionFailure::NotReady,
        PluginFailure::Invalid | PluginFailure::SettingsInvalid => ActionFailure::InvalidInput,
        _ => ActionFailure::Upstream,
    }
}
