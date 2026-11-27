use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Weak};
use std::time::Duration;

use tokio::sync::watch::Receiver;
use wonton_protocol::{
    HomeStatus, InstalledPlugins, PluginFailure, PluginOperation, PluginsViewModel,
};

use super::{OnboardingLink, api, lock, review};

#[derive(Default)]
struct PluginChanges {
    installed: Option<InstalledPlugins>,
    tasks: HashMap<String, (PluginOperation, bool)>,
}

impl PluginChanges {
    fn observe(&mut self, view: &PluginsViewModel) -> bool {
        let mut changed = self.installed != view.installed;
        if changed {
            self.installed.clone_from(&view.installed);
        }
        self.tasks.retain(|id, (_, running)| {
            let present = view.tasks.iter().any(|task| task.id == *id);
            changed |= *running && !present;
            present
        });
        for task in &view.tasks {
            let running = task.state.in_progress();
            let next = (task.operation, running);
            if let Some(previous) = self.tasks.get_mut(&task.id) {
                changed |= !running && *previous != next;
                *previous = next;
            } else {
                // watch 可能把 Running 和结束合成一次通知；新出现的终态也算完成。
                changed |= !running;
                self.tasks.insert(task.id.clone(), next);
            }
        }
        changed
    }
}

pub(super) async fn follow_plugins(
    link: Weak<OnboardingLink>,
    mut plugins: Receiver<PluginsViewModel>,
) {
    let mut changes = PluginChanges::default();
    changes.observe(&plugins.borrow_and_update());
    while plugins.changed().await.is_ok() {
        let Some(link) = link.upgrade() else { return };
        let mut changed = false;
        loop {
            // 拿着状态锁再取视图：写那一下用的总是最新一份。克隆之后才加锁的话，旧快照会落在重试把它改成
            // `Waiting` 之后，把刚起来的安装写回上一次的失败（两边的锁不一样，中间隔着一次发布）。
            let touched = {
                let mut state = lock(&link.state);
                let view = plugins.borrow_and_update().clone();
                changed |= changes.observe(&view);
                review::follow_tasks(&mut state.review, &view.tasks)
            };
            // 点完成以后的安装进展落到最后一页的项上。
            if touched {
                link.publish();
            }
            // 处理期间又出了一版：接着处理新的那版，不把旧的当结论停下。
            if !plugins.has_changed().unwrap_or(false) {
                break;
            }
        }
        if changed {
            refresh(&link).await;
        }
    }
}

async fn refresh(link: &Arc<OnboardingLink>) {
    let (generation, operation, revision) = {
        let mut state = lock(&link.state);
        if state.onboarding.is_none() {
            let retry = !state.loading && link.home.view().status == HomeStatus::Connected;
            drop(state);
            if retry {
                link.refresh_automatically();
            }
            return;
        }
        state.bot_refreshing = true;
        (
            link.generation.load(Ordering::SeqCst),
            link.operation.load(Ordering::SeqCst),
            state.revision,
        )
    };
    link.publish();
    let result = match link.plugins.api() {
        Ok(api) => api::get(&api).await,
        Err(problem) => Err(problem),
    };
    {
        let mut state = lock(&link.state);
        if !link.operation_is_current(generation, operation) || state.revision != revision {
            return;
        }
        state.bot_refreshing = link.home.view().status != HomeStatus::Connected;
        match result {
            Ok(onboarding) => {
                state.onboarding = Some(onboarding);
                state.bot_problem = None;
                state.transport_retries = 0;
                state.revision += 1;
            }
            Err(problem) => state.bot_problem = Some(problem),
        }
    }
    link.publish();
    link.retry_while_connected();
}

/// 自己再取的第一次等多久；之后每次翻倍，到 [`RETRY_MAX`] 为止。
const RETRY_FIRST: Duration = Duration::from_secs(2);
const RETRY_MAX: Duration = Duration::from_secs(60);

impl OnboardingLink {
    /// 连着却没取到：隧道换了连接、请求断在半路，连接状态一直是“已连接”。跟着连接的重取只在连上的那一刻发生，
    /// 这时不会再来，提示就一直挂着（主人看到“连不上家里的服务器”，旁边写着已连接）。所以连着的时候自己隔一会儿
    /// 再取，间隔翻倍到一分钟，取到了提示就收掉；断开时不排，等连上时那一趟。
    pub(super) fn retry_while_connected(self: &Arc<Self>) {
        let generation = self.generation();
        let delay = {
            let mut state = lock(&self.state);
            let failed = |problem: &Option<wonton_protocol::PluginProblem>| {
                problem
                    .as_ref()
                    .is_some_and(|problem| problem.reason == PluginFailure::TransportUnavailable)
            };
            if state.retry_scheduled || !(failed(&state.problem) || failed(&state.bot_problem)) {
                return;
            }
            state.retry_scheduled = true;
            let attempt = state.transport_retries.min(6);
            state.transport_retries = state.transport_retries.saturating_add(1);
            RETRY_FIRST.saturating_mul(1 << attempt).min(RETRY_MAX)
        };
        let link = Arc::downgrade(self);
        self.runtime.spawn(async move {
            tokio::time::sleep(delay).await;
            let Some(link) = link.upgrade() else { return };
            let loading = {
                let mut state = lock(&link.state);
                state.retry_scheduled = false;
                state.loading
            };
            if link.generation() == generation
                && !loading
                && link.home.view().status == HomeStatus::Connected
            {
                link.refresh_automatically();
            }
        });
    }
}
