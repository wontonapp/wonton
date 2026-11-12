//! 系统入口的快照（ADR-0100）接到核心上：每个作用域起一个写入任务，换服务器时先清空。

use std::sync::Arc;

use wonton_protocol::ServerKind;

use super::ClientCore;
use crate::scope::Scope;
use crate::subscription::WidgetSnapshotSubscription;
use crate::widget::{Inputs, run};

#[uniffi::export]
impl ClientCore {
    /// 订阅写下的系统入口快照：每写一份来一次。快照文件本身入口用 `read_widget_snapshot` 读。
    pub fn subscribe_widget_snapshot(&self) -> Arc<WidgetSnapshotSubscription> {
        Arc::new(WidgetSnapshotSubscription::new(self.widget.subscribe()))
    }
}

impl ClientCore {
    /// 跟着这个作用域的 ViewModel 写快照。
    pub(super) fn follow_widget(&self, scope: &Scope) {
        let external = self
            .servers
            .view()
            .servers
            .iter()
            .any(|row| row.id == scope.slot && matches!(row.kind, ServerKind::Matrix { .. }));
        let views = self.views();
        let inputs = Inputs {
            home: views.home.subscribe(),
            chat: views.chat.subscribe(),
            onboarding: views.onboarding.subscribe(),
            plugins: views.plugins.subscribe(),
            appearance: self.appearance.subscribe(),
            external,
        };
        scope.spawn(run(self.widget.clone(), inputs));
    }

    /// 换了服务器：上一台的会话立刻从快照里清掉。
    pub(super) fn clear_widget_for_switch(&self) {
        self.widget.switching(&self.appearance.view());
    }
}
