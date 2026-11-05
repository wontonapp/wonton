use std::collections::HashMap;
use std::sync::Weak;
use std::sync::atomic::Ordering;

use tokio::sync::watch::Receiver;
use wonton_protocol::{InstalledPlugins, PluginOperation, PluginTaskState, PluginsViewModel};

use super::{OnboardingLink, api, lock};

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
        let changed = changes.observe(&plugins.borrow_and_update());
        let Some(link) = link.upgrade() else { return };
        if changed {
            refresh(&link).await;
        }
    }
}

async fn refresh(link: &OnboardingLink) {
    let (generation, operation, revision) = {
        let state = lock(&link.state);
        if state.onboarding.is_none() {
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
