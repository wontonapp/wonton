use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use super::super::bot_activity::Activity;
use tokio::runtime::Handle;
use tokio::sync::{Notify, watch};
use wonton_matrix::MatrixSession;
use wonton_protocol::{
    BotActivityFacts, BotRef, ChatViewModel, HomeStatus, InstalledPlugins, SharedBot,
};

use super::super::layout::Layouts;
use super::super::lock;
#[cfg(test)]
use super::own::OwnBotFetch;
use super::own::OwnEpoch;
use super::profile::follow_plugins;
use crate::home::HomeLink;
use crate::plugins::PluginsLink;

pub(in crate::chat) struct Bots {
    pub runtime: Handle,
    pub home: Arc<HomeLink>,
    pub plugins: Arc<PluginsLink>,
    pub view: watch::Sender<ChatViewModel>,
    pub session: watch::Sender<Option<Arc<MatrixSession>>>,
    pub layouts: Arc<Layouts>,
    pub foreground: watch::Sender<bool>,
    pub changed: Notify,
    pub epoch: AtomicU64,
    /// 本人 Bot 读取的发布资格。`epoch` 防迟到结果，`retired` 防交接后的旧链接再发有效代。
    pub own_epoch: Mutex<OwnEpoch>,
    /// 测试接缝：真 `MatrixSession` 本地造不出（login/whoami/SDK restore）。装上后 refresh 一律走取数分支，
    /// 等于会话恒在；闭包自己走真实 HTTP 取数。没装时照旧按会话有无分派。
    #[cfg(test)]
    pub(in crate::chat) own_bot_fetch: Mutex<Option<OwnBotFetch>>,
    /// 测试接缝的完成回执：取数任务取走，发布闸门判定完发一次；用例等它定序，不靠固定时长。
    #[cfg(test)]
    pub(in crate::chat) own_bot_fetch_done: Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
    pub refresh_serial: AtomicU64,
    pub local: Mutex<Local>,
}

#[derive(Default)]
pub(in crate::chat) struct Local {
    pub installed: Option<InstalledPlugins>,
    pub records: HashMap<BotRef, Record>,
    pub report: Option<wonton_protocol::BotActivityReport>,
    pub web_generation: u64,
    /// 核心的 Bot 可达清单。`None` 是还没取到或旧核心没有这个接口，退回按活动快照猜。
    pub shared: Option<Vec<SharedBot>>,
    /// 每次取清单的代号：迟到的旧响应不盖新结果。
    pub shared_serial: u64,
}

impl Local {
    /// Bot 名单就绪没有：已安装表和第一份活动报告都到了才认得全名单——每个插件的 default Bot 在
    /// 安装表里，家人自己的 Bot 只在报告里；只有安装表时是「只认识 default」，不是「全部」。
    fn ready(&self) -> bool {
        self.installed.is_some() && self.report.is_some()
    }

    #[cfg(test)]
    pub(in crate::chat) fn clear_runtime(&mut self) {
        self.report = None;
        self.web_generation = self.web_generation.wrapping_add(1);
        for record in self.records.values_mut() {
            record.facts = None;
            record.typing.clear();
            record.activity = Activity::default();
        }
    }
}

pub(in crate::chat) struct Record {
    pub user_id: String,
    pub title: String,
    pub facts: Option<BotActivityFacts>,
    pub profile_rev: Option<u64>,
    pub typing: HashMap<String, bool>,
    pub activity: Activity,
    pub refresh: u64,
    pub identity: u64,
}

impl Bots {
    pub fn new(
        runtime: Handle,
        home: Arc<HomeLink>,
        plugins: Arc<PluginsLink>,
        view: watch::Sender<ChatViewModel>,
        layouts: Arc<Layouts>,
    ) -> Arc<Self> {
        let bots = Arc::new(Self {
            runtime: runtime.clone(),
            home,
            plugins,
            view,
            layouts,
            session: watch::Sender::new(None),
            foreground: watch::Sender::new(true),
            changed: Notify::new(),
            epoch: AtomicU64::new(0),
            own_epoch: Mutex::default(),
            #[cfg(test)]
            own_bot_fetch: Mutex::default(),
            #[cfg(test)]
            own_bot_fetch_done: Mutex::default(),
            local: Mutex::default(),
            refresh_serial: AtomicU64::new(0),
        });
        runtime.spawn(follow_plugins(
            Arc::downgrade(&bots),
            bots.plugins.subscribe(),
        ));
        runtime.spawn(super::super::bot_poll::follow(
            Arc::downgrade(&bots),
            bots.home.subscribe(),
            bots.foreground.subscribe(),
        ));
        runtime.spawn(super::super::bot_typing::follow(
            Arc::downgrade(&bots),
            bots.session.subscribe(),
            bots.view.subscribe(),
            bots.foreground.subscribe(),
        ));
        bots
    }

    /// Bot 名单就绪没有；就绪了「在场的 AI」才算得准（[`Local::ready`]）。
    pub fn ready(&self) -> bool {
        lock(&self.local).ready()
    }

    pub fn set_session(self: &Arc<Self>, session: Option<Arc<MatrixSession>>) {
        self.epoch.fetch_add(1, Ordering::SeqCst);
        {
            // 换人后上一份清单是别人的可达性：先作废，迟到结果也不能回写。
            let mut local = lock(&self.local);
            local.shared = None;
            local.shared_serial = self.refresh_serial.fetch_add(1, Ordering::SeqCst) + 1;
        }
        self.clear_activity();
        self.publish_contacts();
        let issued = {
            let mut own = lock(&self.own_epoch);
            let issued = own.issue();
            self.session.send_replace(session.clone());
            issued
        };
        self.changed.notify_one();
        if let Some(issued) = issued {
            self.refresh_own_private_bot(session, issued);
        }
        self.view.send_modify(|view| {
            for bot in &mut view.bots {
                bot.saving = false;
                bot.problem = None;
            }
        });
        self.refresh_all();
    }

    pub fn foregrounded(self: &Arc<Self>) {
        self.foreground.send_replace(true);
        // 会话快照与代号在同一把锁里取：并发换会话不会把旧会话配到新代上。每次发起都换一代，先前在飞的结果作废。
        let issued = {
            let mut own = lock(&self.own_epoch);
            own.issue()
                .map(|epoch| (self.session.borrow().clone(), epoch))
        };
        if let Some((session, epoch)) = issued {
            self.refresh_own_private_bot(session, epoch);
        }
        self.refresh_all();
    }

    pub fn backgrounded(&self) {
        self.foreground.send_replace(false);
        self.clear_activity();
    }

    pub fn can_poll(&self) -> bool {
        *self.foreground.borrow() && self.home.view().status == HomeStatus::Connected
    }

    pub fn refresh_all(self: &Arc<Self>) {
        let ids: Vec<_> = lock(&self.local).records.keys().cloned().collect();
        for id in ids {
            self.refresh(&id);
        }
        self.refresh_shared();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 行为：已安装表和第一份活动报告都到了，Bot 名单才算就绪；报告被清掉（后台、换会话）又不就绪。
    #[test]
    fn the_bot_list_is_ready_only_when_installed_and_a_report_have_arrived() {
        let mut local = Local::default();
        assert!(!local.ready());
        local.installed = Some(InstalledPlugins {
            plugins: Vec::new(),
            upgrade_check: None,
        });
        assert!(!local.ready());
        local.report = Some(wonton_protocol::BotActivityReport {
            rev: 0,
            bots: Vec::new(),
        });
        assert!(local.ready());
        local.report = None;
        assert!(!local.ready());
    }
}
