use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use wonton_protocol::{HomeStatus, PluginProblem, PluginState, TabRef};

use super::{Listing, POLL_INTERVAL, PluginsLink, lock};

const PAGE_POLL_INTERVAL: Duration = Duration::from_secs(10);

impl PluginsLink {
    pub(crate) fn set_active_page(self: &Arc<Self>, page: Option<&TabRef>) {
        let mut active = lock(&self.active_page);
        if active.as_ref() == page {
            return;
        }
        *active = page.cloned();
        drop(active);
        self.poll_changed.notify_one();
        if page.is_some() && self.can_poll() {
            // 页面刚开就关掉时，这次补取别再发出请求（安静时不应有列表请求）：
            // 查了 active 再发请求放在同一个任务里，中间没有跨任务的空档。
            let link = self.clone();
            self.runtime.spawn(async move {
                if lock(&link.active_page).is_some() {
                    let generation = link.generation.load(Ordering::SeqCst);
                    link.begin_refresh();
                    let result = link.fetch_installed().await;
                    link.end_refresh(Listing::Installed, generation, result);
                }
                link.start_polling();
            });
        }
    }

    // 隧道断着时请求会触发重连；自动检查不能把后台或无连接变成一次主动重连。
    pub(super) fn can_poll(&self) -> bool {
        self.foreground.load(Ordering::SeqCst) && self.home.view().status == HomeStatus::Connected
    }

    pub(super) fn needs_polling(&self) -> Option<Duration> {
        if !self.can_poll() {
            return None;
        }
        let view = self.view.borrow();
        let settling = view.installed.as_ref().is_some_and(|installed| {
            installed.plugins.iter().any(|plugin| {
                matches!(
                    plugin.status,
                    PluginState::Installing | PluginState::Upgrading | PluginState::Uninstalling
                )
            })
        });
        if settling || view.tasks.iter().any(|task| task.state.in_progress()) {
            Some(POLL_INTERVAL)
        } else {
            lock(&self.active_page)
                .is_some()
                .then_some(PAGE_POLL_INTERVAL)
        }
    }

    pub(super) fn start_polling(self: &Arc<Self>) {
        self.poll_changed.notify_one();
        if self.polling.swap(true, Ordering::SeqCst) {
            return;
        }
        let link = self.clone();
        self.runtime.spawn(async move {
            let unreachable = loop {
                let Some(interval) = link.needs_polling() else {
                    break false;
                };
                tokio::select! {
                    () = link.poll_changed.notified() => continue,
                    () = tokio::time::sleep(interval) => {}
                }
                if link.needs_polling().is_none() {
                    break false;
                }
                if link.poll_once().await.is_err() && !link.has_running_task() {
                    break true;
                }
            };
            link.polling.store(false, Ordering::SeqCst);
            // 退出期间可能又切进页面或开始任务；请求失败时不能反复重连。
            if link.needs_polling().is_some() && (!unreachable || link.has_running_task()) {
                link.start_polling();
            }
        });
    }

    fn has_running_task(&self) -> bool {
        self.view
            .borrow()
            .tasks
            .iter()
            .any(|task| task.state.in_progress())
    }

    async fn poll_once(&self) -> Result<(), PluginProblem> {
        let generation = self.generation.load(Ordering::SeqCst);
        let server = self.home.view().server_id;
        // 序号从 1 开始，和 fetch_installed_from 一致。
        let seq = self.fetch_seq.fetch_add(1, Ordering::SeqCst) + 1;
        let result = self.list_installed().await;
        if !self.is_current_home(generation, &server) {
            return Ok(());
        }
        // 和 fetch_installed_from 一样丢掉晚到的旧响应；轮询写完也要把派生字段（横幅数、侧栏）跟着重算。
        let outcome = result.map(|installed| {
            if seq > self.applied_seq.load(Ordering::SeqCst) {
                self.applied_seq.store(seq, Ordering::SeqCst);
                self.view.send_modify(|view| {
                    view.installed = Some(installed);
                    Self::derive(view);
                });
            }
        });
        self.record(Listing::Installed, outcome.clone());
        outcome
    }
}
