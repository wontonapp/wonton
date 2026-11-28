use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::Instant;

use tokio::sync::watch;
use wonton_protocol::ChatViewModel;

use super::super::lock;
use super::SETTLE;
use super::rows::rows;
use crate::plugins::PluginsLink;

pub(in crate::chat) struct Installs {
    plugins: Arc<PluginsLink>,
    view: watch::Sender<ChatViewModel>,
    /// 装好、开始等私聊的插件和时刻。
    settling: Mutex<HashMap<String, Instant>>,
    /// 已经有一个计时器在等超时，不再重复起。
    timer: AtomicBool,
}

impl Installs {
    pub(in crate::chat) fn new(
        plugins: Arc<PluginsLink>,
        view: watch::Sender<ChatViewModel>,
    ) -> Arc<Self> {
        let installs = Arc::new(Self {
            plugins,
            view,
            settling: Mutex::default(),
            timer: AtomicBool::new(false),
        });
        installs.runtime_spawn_follow();
        installs
    }

    fn runtime_spawn_follow(self: &Arc<Self>) {
        let weak = Arc::downgrade(self);
        let plugins = self.plugins.clone();
        self.plugins.runtime().spawn(async move {
            follow(weak, plugins.subscribe()).await;
        });
    }

    /// 房间列表或插件状态变了：重算这一组行。
    pub(in crate::chat) fn refresh(self: &Arc<Self>) {
        let plugins = self.plugins.view();
        let mut settling = lock(&self.settling);
        let now = Instant::now();
        let rows = rows(
            plugins.installed.as_ref(),
            &plugins.tasks,
            &self.plugins.previews(),
            &self.view.borrow().rooms,
            &mut settling,
            now,
        );
        let waiting = settling
            .values()
            .any(|since| now.duration_since(*since) < SETTLE);
        // 装好以后列表不再刷，要自己到点重算一次，不然那一行会一直挂着。
        if waiting && !self.timer.swap(true, Ordering::SeqCst) {
            let installs = Arc::downgrade(self);
            self.plugins.runtime().spawn(async move {
                tokio::time::sleep(SETTLE).await;
                if let Some(installs) = installs.upgrade() {
                    installs.timer.store(false, Ordering::SeqCst);
                    installs.refresh();
                }
            });
        }
        // 插件表变得频繁，行没变时别惊动壳：时间线订阅跟着发批次，空转时一秒几千批会把主线程占死。
        self.view.send_if_modified(|view| {
            let changed = super::super::plugin_status::refresh(&mut view.rooms, &plugins);
            if view.installing_bots == rows {
                return changed;
            }
            view.installing_bots = rows;
            true
        });
    }
}

async fn follow(
    installs: Weak<Installs>,
    mut plugins: watch::Receiver<wonton_protocol::PluginsViewModel>,
) {
    loop {
        if let Some(installs) = installs.upgrade() {
            installs.refresh();
        } else {
            return;
        }
        if plugins.changed().await.is_err() {
            return;
        }
    }
}
