use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Weak};

use tokio::sync::watch::Receiver;
use wonton_protocol::{
    HomeStatus, InstalledPlugins, PluginOperation, PluginTaskState, PluginsViewModel,
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
            let running = task.state == PluginTaskState::Running;
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
        let state = lock(&link.state);
        if state.onboarding.is_none() {
            // 第一次取状态失败（配对后家里的服务器还没就绪）时不能就此停住：插件那头能连上了就再取一次（#434）。
            let retry = !state.loading && link.home.view().status == HomeStatus::Connected;
            drop(state);
            if retry {
                link.refresh();
            }
            return;
        };
        (
            link.generation.load(Ordering::SeqCst),
            link.operation.load(Ordering::SeqCst),
            state.revision,
        )
    };
    let Ok(api) = link.plugins.api() else {
        return;
    };
    let Ok(onboarding) = api::get(&api).await else {
        return;
    };
    {
        let mut state = lock(&link.state);
        // begin_operation 和换服务器也持这把锁，检查与写入之间不能开始新操作。
        // 已经在保存的操作可能先完成；它的响应同样不能被这个 GET 倒退。
        if !link.operation_is_current(generation, operation) || state.revision != revision {
            return;
        }
        state.onboarding = Some(onboarding);
        state.revision += 1;
    }
    link.publish();
    link.maybe_mark_prompt_used();
}
