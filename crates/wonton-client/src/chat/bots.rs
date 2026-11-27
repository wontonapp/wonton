use std::collections::{HashMap, HashSet};
#[cfg(test)]
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};

use tokio::runtime::Handle;
use tokio::sync::{Notify, watch};
use tokio::time::Instant;
use wonton_matrix::{MatrixSession, UserProfile};
use wonton_protocol::{
    AVATARS_FIELD, AvatarImage, AvatarKind, AvatarMedia, BotAccess, BotActivity, BotActivityFacts,
    BotChatAvailability, BotChatUnavailable, BotRef, BotView, ChatForm, ChatViewModel, CoreError,
    HomeStatus, InstalledPlugins, MediaHandle, RoomBot, SharedBot, SharedBotsState, SharedBotsView,
    StateAvatar, StateAvatars,
};

use super::bot_activity::{Activity, synthesize};
use super::contacts;
use super::layout::{self, Layouts};
use super::lock;
use crate::home::HomeLink;
use crate::plugins::PluginsLink;

pub(super) struct Bots {
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
    pub(super) own_bot_fetch: Mutex<Option<OwnBotFetch>>,
    /// 测试接缝的完成回执：取数任务取走，发布闸门判定完发一次；用例等它定序，不靠固定时长。
    #[cfg(test)]
    pub(super) own_bot_fetch_done: Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
    pub refresh_serial: AtomicU64,
    pub local: Mutex<Local>,
}

#[derive(Default)]
pub(super) struct OwnEpoch {
    epoch: u64,
    retired: bool,
    // 同步生命周期没有 await；测试在捕获代号后、检查写槽资格前暂停一次真实调用。
    #[cfg(test)]
    pub(super) before_refresh: Option<Box<dyn FnOnce() + Send>>,
}

/// 测试接缝的取数闭包：每次调用返回一次真实取数，结果原样交给发布闸门。
#[cfg(test)]
pub(super) type OwnBotFetch = Arc<
    dyn Fn() -> Pin<
            Box<
                dyn Future<
                        Output = Result<
                            wonton_protocol::OwnPrivateBotView,
                            wonton_protocol::OwnBotProblem,
                        >,
                    > + Send,
            >,
        > + Send
        + Sync,
>;

impl OwnEpoch {
    #[cfg(test)]
    pub(super) fn current(&self) -> u64 {
        self.epoch
    }

    /// 领取有效代；退役后不再发号。
    fn issue(&mut self) -> Option<u64> {
        if self.retired {
            return None;
        }
        self.epoch = self.epoch.wrapping_add(1);
        Some(self.epoch)
    }

    /// 只有当前且未退役的代能写槽。
    fn accepts(&self, epoch: u64) -> bool {
        !self.retired && self.epoch == epoch
    }
}

impl Bots {
    /// 不可逆退役：此后本链接不再发有效代，迟到的结果与清槽都被拒。
    pub(super) fn retire_own_private_bot(&self) {
        let mut own = lock(&self.own_epoch);
        own.epoch = own.epoch.wrapping_add(1);
        own.retired = true;
    }
}

#[derive(Default)]
pub(super) struct Local {
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
    pub(super) fn clear_runtime(&mut self) {
        self.report = None;
        self.web_generation = self.web_generation.wrapping_add(1);
        for record in self.records.values_mut() {
            record.facts = None;
            record.typing.clear();
            record.activity = Activity::default();
        }
    }
}

pub(super) struct Record {
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
        runtime.spawn(super::bot_poll::follow(
            Arc::downgrade(&bots),
            bots.home.subscribe(),
            bots.foreground.subscribe(),
        ));
        runtime.spawn(super::bot_typing::follow(
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

    /// 本人的私人 Bot 身份是独立读取：成功写身份槽并清掉问题，失败只写本人问题槽，不占全局 `problem`。
    /// 有效性比对和写入在同一把 `own_epoch` 锁里，换会话或注销后迟到的结果不写回旧身份。
    pub fn refresh_own_private_bot(
        self: &Arc<Self>,
        session: Option<Arc<MatrixSession>>,
        epoch: u64,
    ) {
        // 接缝在暂停前快照：暂停期间装上或换掉的闭包不影响这次调用已经选定的分派。
        #[cfg(test)]
        let fetch = lock(&self.own_bot_fetch).clone();
        #[cfg(test)]
        {
            let pause = lock(&self.own_epoch).before_refresh.take();
            if let Some(pause) = pause {
                pause();
            }
        }
        #[cfg(test)]
        if let Some(fetch) = fetch {
            self.spawn_own_bot_fetch(fetch, epoch);
            return;
        }
        let Some(session) = session else {
            let own = lock(&self.own_epoch);
            if !own.accepts(epoch) {
                return;
            }
            self.view.send_modify(|view| {
                view.own_private_bot = None;
                view.own_private_bot_problem = None;
            });
            return;
        };
        let weak = Arc::downgrade(self);
        let plugins = self.plugins.clone();
        self.runtime.spawn(async move {
            let outcome = plugins.own_private_bot(&session).await;
            let Some(bots) = weak.upgrade() else {
                return;
            };
            bots.publish_own_private_bot(epoch, outcome);
        });
    }

    /// 接缝取数的后台任务：和会话取数是同一条发布闸门，闭包替代了会话上的取数。
    /// 一次性取走完成回执，在发布闸门判定完才发：用例据此确认旧任务已经过闸门，不等固定时长。
    #[cfg(test)]
    fn spawn_own_bot_fetch(self: &Arc<Self>, fetch: OwnBotFetch, epoch: u64) {
        let weak = Arc::downgrade(self);
        let done = lock(&self.own_bot_fetch_done).take();
        self.runtime.spawn(async move {
            let outcome = fetch().await;
            let Some(bots) = weak.upgrade() else {
                return;
            };
            bots.publish_own_private_bot(epoch, outcome);
            if let Some(done) = done {
                let _ = done.send(());
            }
        });
    }

    /// 结果发布：代比对与双槽写入在同一把锁里，换会话／注销后迟到的结果不写回旧身份。
    pub(super) fn publish_own_private_bot(
        &self,
        epoch: u64,
        outcome: Result<wonton_protocol::OwnPrivateBotView, wonton_protocol::OwnBotProblem>,
    ) {
        let own = lock(&self.own_epoch);
        if !own.accepts(epoch) {
            return;
        }
        self.view.send_modify(|view| match outcome {
            Ok(own) => {
                view.own_private_bot = Some(own);
                view.own_private_bot_problem = None;
            }
            Err(problem) => view.own_private_bot_problem = Some(problem),
        });
        drop(own);
        self.publish_contacts();
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

    /// 取当前人的 Bot 可达清单：单飞、按代号丢迟到结果；失败时留上一份，不清空。
    fn refresh_shared(self: &Arc<Self>) {
        if self.session.borrow().is_none() {
            return;
        }
        let serial = {
            let mut local = lock(&self.local);
            local.shared_serial = self.refresh_serial.fetch_add(1, Ordering::SeqCst) + 1;
            local.shared_serial
        };
        let plugins = self.plugins.clone();
        let bots = self.clone();
        self.runtime.spawn(async move {
            let outcome = plugins.shared_bots().await;
            let mut local = lock(&bots.local);
            if !Self::apply_shared_result(&mut local, serial, outcome) {
                return;
            }
            drop(local);
            bots.publish_contacts();
            // 多个等待者（房间行、活动刷新）都该重算；notify_one 只叫醒一个。
            bots.changed.notify_waiters();
        });
    }

    /// 把一次清单结果写进本地。代号不匹配（换过人/换过会话）和取数失败都保留旧值；
    /// 旧核心的 404 是 `Ok(None)`，清空而不是当失败。整份不可信时行按 Unknown 用。
    fn apply_shared_result(
        local: &mut Local,
        serial: u64,
        outcome: Result<Option<SharedBotsView>, CoreError>,
    ) -> bool {
        if local.shared_serial != serial {
            return false;
        }
        match outcome {
            Ok(Some(view)) => local.shared = Some(Self::rows_from_view(view)),
            Ok(None) => local.shared = None,
            // 取不到就留上一份，界面别因为一次断线把能选的 Bot 全撤掉。
            Err(_) => return false,
        }
        true
    }

    /// 整份清单不可信时每一行都按 Unknown 用：壳照常能发，但不把上一轮的结论当本轮。
    fn rows_from_view(mut view: SharedBotsView) -> Vec<SharedBot> {
        if view.state != SharedBotsState::Ready {
            for row in &mut view.bots {
                // 缺记录是“已经知道”的事实，不随整份清单不可信一起抹掉。
                if matches!(
                    row.access,
                    BotAccess::Unusable {
                        reason: BotChatUnavailable::BotMissing
                    }
                ) {
                    continue;
                }
                row.access = BotAccess::Unknown;
            }
        }
        view.bots
    }

    /// 房间里的对方是 Bot 时给出身份和可用性：以核心的可达清单为准；外站 Bot、旧核心或清单还没到时退回活动快照，
    /// 退不出仍是 `None`（当普通人，输入照常）。
    pub(super) fn room_bot(
        &self,
        user_id: Option<&str>,
        room_name: &str,
        name_fallback: bool,
    ) -> Option<RoomBot> {
        let shared = lock(&self.local);
        let row = user_id
            .and_then(|user_id| {
                shared
                    .shared
                    .as_ref()?
                    .iter()
                    .find(|row| row.user_id.as_deref() == Some(user_id))
            })
            .or_else(|| {
                if !name_fallback {
                    return None;
                }
                // 记录没建出来的行没有 Matrix ID，按插件显示名和房间名对上；同名时只当提示，不当发送资格。
                shared.shared.as_ref()?.iter().find(|row| {
                    row.user_id.is_none()
                        && matches!(
                            row.access,
                            BotAccess::Unusable {
                                reason: BotChatUnavailable::BotMissing
                            }
                        )
                        && row.title == room_name
                })
            });
        if let Some(row) = row {
            return Some(RoomBot {
                bot: row.bot.clone(),
                // 缺记录的行没有自己的 Matrix ID：用房间里一直记着的那一个，发消息/建私聊不空着。
                user_id: row
                    .user_id
                    .clone()
                    .unwrap_or_else(|| user_id.unwrap_or_default().to_owned()),
                title: row.title.clone(),
                access: row.access,
            });
        }
        drop(shared);
        let view = self.view.borrow();
        let bot = user_id
            .and_then(|user_id| view.bots.iter().find(|bot| bot.user_id == user_id))
            .or_else(|| {
                // 旧核心/联邦对不上 ID 时，只在私聊里按显示名找一次；找不到仍是普通人。
                name_fallback
                    .then(|| view.bots.iter().find(|bot| bot.name == room_name))
                    .flatten()
            })?;
        Some(RoomBot {
            bot: bot.bot.clone(),
            user_id: bot.user_id.clone(),
            title: bot.name.clone(),
            access: bot.availability.into(),
        })
    }

    /// 把当前能选的人合成进 ViewModel；壳只按搜索词过滤，不自己拼。
    pub(super) fn publish_contacts(&self) {
        let contacts = {
            let local = lock(&self.local);
            let view = self.view.borrow();
            contacts::assemble(
                &view.bots,
                local.shared.as_deref(),
                &view.rooms,
                view.own_private_bot.as_ref(),
            )
        };
        self.view.send_if_modified(|view| {
            if view.shared_contacts == contacts {
                return false;
            }
            view.shared_contacts = contacts;
            true
        });
    }

    pub fn refresh(self: &Arc<Self>, id: &BotRef) {
        let Some(session) = self.session.borrow().clone() else {
            return;
        };
        let (user, serial) = {
            let mut local = lock(&self.local);
            let Some(record) = local.records.get_mut(id) else {
                return;
            };
            record.refresh = self.refresh_serial.fetch_add(1, Ordering::SeqCst) + 1;
            (record.user_id.clone(), record.refresh)
        };
        let Ok(user_id) = user.parse::<wonton_matrix::OwnedUserId>() else {
            tracing::warn!(user_id = user, "Invalid installed Bot user ID");
            return;
        };
        let epoch = self.epoch.load(Ordering::SeqCst);
        let id = id.clone();
        let bots = self.clone();
        self.runtime.spawn(async move {
            let profile = session.fetch_user_profile(&user_id).await;
            if bots.epoch.load(Ordering::SeqCst) != epoch
                || !bots
                    .session
                    .borrow()
                    .as_ref()
                    .is_some_and(|current| Arc::ptr_eq(current, &session))
            {
                return;
            }
            let local = lock(&bots.local);
            let Some(record) = local
                .records
                .get(&id)
                .filter(|record| record.user_id == user && record.refresh == serial)
            else {
                return;
            };
            let (name, avatar, state_avatars) = match profile {
                Ok(profile) => profile_view(profile, &record.title),
                Err(error) => {
                    tracing::info!(plugin_id = id.plugin_id, bot = id.bot, %error, "Cannot fetch Bot profile");
                    (record.title.clone(), None, Vec::new())
                }
            };
            bots.view.send_modify(|view| {
                if let Some(bot) = view.bots.iter_mut().find(|bot| bot.bot == id && bot.user_id == user) {
                    bot.name = name;
                    bot.avatar = avatar;
                    bot.state_avatars = state_avatars;
                }
            });
        });
    }

    #[allow(clippy::too_many_lines)] // 一次重建就是把插件表、旧视图和快照各自对一遍，拆开要来回传同一批临时表。
    fn replace_plugins(self: &Arc<Self>, installed: Option<InstalledPlugins>) {
        let mut local = lock(&self.local);
        if local.installed == installed {
            return;
        }
        let mut records = std::mem::take(&mut local.records);
        let old = self.view.borrow().bots.clone();
        let mut views = Vec::new();
        let mut covered = HashSet::new();
        if let Some(installed) = &installed {
            for plugin in &installed.plugins {
                let Some(user) = plugin.bot_user_id.as_ref().filter(|user| !user.is_empty()) else {
                    // 身份暂时读不明（断线、切换账号）：这个插件已开的私聊照留，别让列表和记录断掉。
                    for bot in old.iter().filter(|bot| bot.bot.plugin_id == plugin.id) {
                        if !covered.insert(bot.user_id.clone()) {
                            continue;
                        }
                        if let Some(record) = records
                            .remove(&bot.bot)
                            .filter(|record| record.user_id == bot.user_id)
                        {
                            local.records.insert(bot.bot.clone(), record);
                        }
                        views.push(bot.clone());
                    }
                    continue;
                };
                let key = BotRef {
                    plugin_id: plugin.id.clone(),
                    bot: "default".into(),
                };
                let previous = records
                    .remove(&key)
                    .filter(|record| record.user_id == *user);
                let mut bot = previous
                    .as_ref()
                    .and_then(|_| {
                        old.iter()
                            .find(|bot| bot.bot == key && bot.user_id == *user)
                    })
                    .cloned()
                    .unwrap_or_else(|| BotView {
                        chat: Default::default(),
                        user_id: user.clone(),
                        availability: Default::default(),
                        bot: BotRef {
                            plugin_id: plugin.id.clone(),
                            bot: "default".into(),
                        },
                        web: None,
                        web_page: None,
                        name: plugin.title.clone(),
                        avatar: None,
                        state_avatars: Vec::new(),
                        activity: None,
                        saving: false,
                        problem: None,
                    });
                bot.chat = layout::bot_settings(
                    &self.layouts.current(),
                    plugin.chat.as_ref(),
                    bot.user_id.as_str(),
                );
                let mut record = previous.unwrap_or_else(|| Record {
                    user_id: user.clone(),
                    title: plugin.title.clone(),
                    facts: None,
                    profile_rev: None,
                    typing: HashMap::new(),
                    activity: Activity::default(),
                    refresh: 0,
                    identity: self.refresh_serial.fetch_add(1, Ordering::SeqCst) + 1,
                });
                record.title.clone_from(&plugin.title);
                record.facts = local
                    .report
                    .as_ref()
                    .and_then(|report| {
                        report
                            .bots
                            .iter()
                            .find(|facts| facts.plugin_id == plugin.id && facts.user_id == *user)
                    })
                    .cloned();
                if let Some(facts) = record.facts.as_ref() {
                    bot.availability = facts.chat;
                }
                covered.insert(user.clone());
                local.records.insert(key, record);
                views.push(bot);
            }
        }
        // 插件表一变，插件响应可能暂时没给这个人的 Bot 身份（会话刚接上、读不明）：活动快照里有、
        // 这次没覆盖到的 Bot 也留着，房间行的原因和禁用别断到下一次快照。
        let reported = local
            .report
            .clone()
            .map(|report| report.bots)
            .unwrap_or_default();
        for facts in reported {
            self.restore_reported(
                facts,
                &mut records,
                &old,
                installed.as_ref(),
                &mut covered,
                &mut views,
            );
        }
        local.installed = installed;
        // 插件表变得频繁，Bot 列表没变时别惊动壳：发送方每次通知都会变成一批时间线增量。
        self.view.send_if_modified(|view| {
            if view.bots == views {
                return false;
            }
            view.bots = views;
            true
        });
        drop(local);
        self.publish_contacts();
        self.update_activity();
        self.refresh_all();
        self.changed.notify_one();
        self.layouts.touch();
    }

    /// 把活动快照里、这次插件响应没覆盖的 Bot 原样接回来；身份、原因和资料都照旧。
    fn restore_reported(
        &self,
        facts: BotActivityFacts,
        records: &mut HashMap<BotRef, Record>,
        old: &[BotView],
        installed: Option<&InstalledPlugins>,
        covered: &mut HashSet<String>,
        views: &mut Vec<BotView>,
    ) {
        if !covered.insert(facts.user_id.clone()) {
            return;
        }
        let key = BotRef {
            plugin_id: facts.plugin_id.clone(),
            bot: facts.bot.clone().unwrap_or_else(|| "default".into()),
        };
        let plugin = installed.and_then(|installed| {
            installed
                .plugins
                .iter()
                .find(|plugin| plugin.id == facts.plugin_id)
        });
        let title = plugin.map_or_else(|| facts.plugin_id.clone(), |plugin| plugin.title.clone());
        let previous = records
            .remove(&key)
            .filter(|record| record.user_id == facts.user_id);
        let mut bot = old
            .iter()
            .find(|bot| bot.bot == key && bot.user_id == facts.user_id)
            .cloned()
            .unwrap_or_else(|| BotView {
                chat: Default::default(),
                user_id: facts.user_id.clone(),
                availability: Default::default(),
                bot: key.clone(),
                web: None,
                web_page: None,
                name: title.clone(),
                avatar: None,
                state_avatars: Vec::new(),
                activity: None,
                saving: false,
                problem: None,
            });
        bot.availability = facts.chat;
        bot.chat = layout::bot_settings(
            &self.layouts.current(),
            plugin.and_then(|plugin| plugin.chat.as_ref()),
            &facts.user_id,
        );
        let mut record = previous.unwrap_or_else(|| Record {
            user_id: facts.user_id.clone(),
            title,
            facts: None,
            profile_rev: None,
            typing: HashMap::new(),
            activity: Activity::default(),
            refresh: 0,
            identity: self.refresh_serial.fetch_add(1, Ordering::SeqCst) + 1,
        });
        record.facts = Some(facts);
        records.insert(key, record);
        views.push(bot);
    }

    /// 这个 Bot 所属插件声明的显示形态；不是插件 Bot 时 `None`。
    pub fn chat_form(&self, user: &str) -> Option<ChatForm> {
        let local = lock(&self.local);
        local
            .installed
            .as_ref()?
            .plugins
            .iter()
            .find(|plugin| plugin.bot_user_id.as_deref() == Some(user))
            .and_then(|plugin| plugin.chat.as_ref())
            .map(|chat| chat.form)
    }

    /// 账号数据里的选择变了：重算每个 Bot 资料页的“显示”。
    pub fn refresh_chat(&self) {
        let settings = self.layouts.current();
        let local = lock(&self.local);
        let Some(installed) = &local.installed else {
            return;
        };
        self.view.send_modify(|view| {
            for bot in &mut view.bots {
                let plugin = installed
                    .plugins
                    .iter()
                    .find(|plugin| plugin.id == bot.bot.plugin_id);
                bot.chat = layout::bot_settings(
                    &settings,
                    plugin.and_then(|plugin| plugin.chat.as_ref()),
                    bot.user_id.as_str(),
                );
            }
        });
    }

    pub fn clear_activity(&self) {
        let mut local = lock(&self.local);
        local.report = None;
        // 后台或换会话时网页状态作废：换一代，把已发的网页地址撤下来。
        local.web_generation = local.web_generation.wrapping_add(1);
        self.view.send_modify(|view| {
            for bot in &mut view.bots {
                bot.web = None;
                bot.web_page = None;
                // 没新的快照就核实不了资格：不再声称“能用”或“不能用”，壳照常能发、只写还没核实。
                bot.availability = BotChatAvailability::Unknown;
            }
        });
        for record in local.records.values_mut() {
            record.facts = None;
            record.typing.clear();
            record.activity = synthesize(&record.activity, None, false, Instant::now());
        }
        self.publish_activity(&local);
        self.changed.notify_one();
    }

    pub fn update_activity(&self) -> Option<Instant> {
        let mut local = lock(&self.local);
        let now = Instant::now();
        for record in local.records.values_mut() {
            record.activity = synthesize(
                &record.activity,
                record.facts.as_ref(),
                record.typing.values().any(|typing| *typing),
                now,
            );
        }
        self.publish_activity(&local);
        local
            .records
            .values()
            .filter_map(|record| record.activity.next_change)
            .min()
    }

    fn publish_activity(&self, local: &Local) {
        self.view.send_if_modified(|view| {
            let mut changed = false;
            for bot in &mut view.bots {
                let activity = local
                    .records
                    .get(&bot.bot)
                    .and_then(|record| record.activity.state);
                changed |= bot.activity != activity;
                bot.activity = activity;
            }
            changed
        });
    }
}

async fn follow_plugins(
    bots: Weak<Bots>,
    mut plugins: watch::Receiver<wonton_protocol::PluginsViewModel>,
) {
    loop {
        let Some(bots) = bots.upgrade() else { return };
        let installed = {
            let view = plugins.borrow_and_update();
            (lock(&bots.local).installed != view.installed).then(|| view.installed.clone())
        };
        if let Some(installed) = installed {
            bots.replace_plugins(installed);
        }
        drop(bots);
        if plugins.changed().await.is_err() {
            return;
        }
    }
}

pub(super) fn handle(url: &str) -> Option<MediaHandle> {
    match wonton_matrix::media_token(url) {
        Ok(token) => Some(MediaHandle { token }),
        Err(error) => {
            tracing::warn!(%error, "Invalid Bot avatar MXC");
            None
        }
    }
}

fn image(media: &AvatarMedia) -> Option<AvatarImage> {
    let kind = if media.mimetype.starts_with("video/") {
        AvatarKind::Video
    } else if media.mimetype == "image/gif"
        || (media.mimetype == "image/webp" && media.duration_ms.is_some())
    {
        AvatarKind::Animated
    } else {
        AvatarKind::Still
    };
    Some(AvatarImage {
        media: handle(&media.url)?,
        kind,
        width: Some(media.width),
        height: Some(media.height),
        poster: media.poster.as_deref().and_then(handle),
    })
}

/// 现在该播的状态头像。壳按它显示，没有时用静态头像 `avatar`，再没有用插件图标。
#[uniffi::export]
pub fn bot_current_avatar(bot: BotView) -> Option<AvatarImage> {
    bot.current_state_avatar().cloned()
}

fn profile_view(
    mut profile: UserProfile,
    fallback: &str,
) -> (String, Option<AvatarImage>, Vec<StateAvatar>) {
    let avatar = profile
        .avatar_url
        .as_deref()
        .and_then(handle)
        .map(|media| AvatarImage {
            media,
            kind: AvatarKind::Still,
            width: None,
            height: None,
            poster: None,
        });
    let states = profile.fields.remove(AVATARS_FIELD).and_then(|value| {
        serde_json::from_value::<StateAvatars>(value)
            .inspect_err(|error| tracing::warn!(%error, "Invalid Bot state avatars"))
            .ok()
    });
    let state_avatars = states.map_or_else(Vec::new, |avatars| {
        BotActivity::ALL
            .into_iter()
            .filter_map(|state| {
                avatars
                    .states
                    .get(state.key())
                    .and_then(image)
                    .map(|image| StateAvatar { state, image })
            })
            .collect()
    });
    (
        profile.display_name.unwrap_or_else(|| fallback.to_owned()),
        avatar,
        state_avatars,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retired_epoch_stops_issuing_and_rejects_late_results() {
        let mut own = OwnEpoch::default();
        let first = own.issue().unwrap();
        let second = own.issue().unwrap();
        assert_eq!(own.current(), second);
        assert!(!own.accepts(first));
        own.retired = true;
        let retired = own.current();
        assert_eq!(own.issue(), None);
        assert_eq!(own.current(), retired);
        assert!(!own.accepts(second));
    }

    fn view(activity: Option<BotActivity>, states: &[(&str, BotActivity)]) -> BotView {
        BotView {
            chat: Default::default(),
            availability: Default::default(),
            user_id: "@bot:home".into(),
            bot: BotRef {
                plugin_id: "app.bot".into(),
                bot: "default".into(),
            },
            web: None,
            web_page: None,
            name: "Bot".into(),
            avatar: None,
            state_avatars: states
                .iter()
                .map(|(url, state)| StateAvatar {
                    state: *state,
                    image: AvatarImage {
                        media: MediaHandle {
                            token: (*url).to_owned(),
                        },
                        kind: AvatarKind::Still,
                        width: None,
                        height: None,
                        poster: None,
                    },
                })
                .collect(),
            activity,
            saving: false,
            problem: None,
        }
    }

    #[test]
    fn the_shell_plays_the_state_avatar_and_falls_back_to_idle() {
        let both = view(
            Some(BotActivity::Working),
            &[
                ("idle", BotActivity::Idle),
                ("working", BotActivity::Working),
            ],
        );
        assert_eq!(bot_current_avatar(both).unwrap().media.token, "working");
        // 没有这个状态的素材时用待机的。
        let idle_only = view(Some(BotActivity::Working), &[("idle", BotActivity::Idle)]);
        assert_eq!(bot_current_avatar(idle_only).unwrap().media.token, "idle");
    }

    #[test]
    fn uncertain_or_unsourced_states_leave_the_choice_to_the_static_avatar() {
        assert_eq!(
            bot_current_avatar(view(None, &[("idle", BotActivity::Idle)]))
                .map(|image| image.media.token),
            None
        );
        assert_eq!(
            bot_current_avatar(view(Some(BotActivity::Working), &[])),
            None
        );
    }

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

#[cfg(test)]
mod shared_rows_tests {
    use super::*;

    fn row(access: BotAccess) -> SharedBot {
        SharedBot {
            bot: BotRef {
                plugin_id: "app.bot".into(),
                bot: "default".into(),
            },
            user_id: Some("@bot:test".into()),
            title: "Bot".into(),
            icon: None,
            access,
        }
    }

    #[test]
    fn not_ready_downgrades_every_row_to_unknown() {
        let view = SharedBotsView {
            state: SharedBotsState::Unknown,
            bots: vec![
                row(BotAccess::Own),
                row(BotAccess::Unusable {
                    reason: wonton_protocol::BotChatUnavailable::AppNotShared,
                }),
            ],
        };
        let rows = Bots::rows_from_view(view);
        assert!(rows.iter().all(|row| row.access == BotAccess::Unknown));
    }

    #[test]
    fn failed_or_stale_results_keep_the_previous_list() {
        let mut local = Local {
            shared: Some(vec![row(BotAccess::Own)]),
            shared_serial: 2,
            ..Default::default()
        };
        assert!(!Bots::apply_shared_result(
            &mut local,
            1,
            Ok(Some(SharedBotsView {
                state: SharedBotsState::Ready,
                bots: vec![row(BotAccess::Shared)],
            })),
        ));
        assert_eq!(local.shared.as_ref().unwrap()[0].access, BotAccess::Own);
        assert!(!Bots::apply_shared_result(
            &mut local,
            2,
            Err(CoreError::Upstream {
                reason: "offline".into(),
            }),
        ));
        assert!(local.shared.is_some());
        assert!(Bots::apply_shared_result(&mut local, 2, Ok(None)));
        assert!(local.shared.is_none());
    }

    #[test]
    fn not_ready_result_writes_rows_as_unknown() {
        let mut local = Local {
            shared_serial: 7,
            ..Default::default()
        };
        assert!(Bots::apply_shared_result(
            &mut local,
            7,
            Ok(Some(SharedBotsView {
                state: SharedBotsState::Unknown,
                bots: vec![row(BotAccess::Shared)],
            })),
        ));
        assert_eq!(local.shared.unwrap()[0].access, BotAccess::Unknown);
    }

    #[test]
    fn ready_keeps_the_reasons() {
        let view = SharedBotsView {
            state: SharedBotsState::Ready,
            bots: vec![row(BotAccess::Shared)],
        };
        assert_eq!(Bots::rows_from_view(view)[0].access, BotAccess::Shared);
    }
}
