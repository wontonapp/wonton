use std::collections::{HashMap, HashSet};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Weak};

use tokio::sync::watch;
use wonton_protocol::{BotActivityFacts, BotRef, BotView, ChatForm, InstalledPlugins};

use super::super::bot_activity::Activity;
use super::super::{layout, lock};
use super::avatar::profile_view;
use super::{Bots, Record};

impl Bots {
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
                        shared: plugin.bot_shared,
                        danger: (plugin.bot_danger || plugin.bot_danger_can_toggle).then_some(
                            wonton_protocol::DangerModeView {
                                enabled: plugin.bot_danger,
                                can_toggle: plugin.bot_danger_can_toggle,
                                reason: None,
                            },
                        ),
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
        // 插件表变得频繁；Bot 列表和联系人候选要一起写，否则壳会看到新 Bot 却还没有联系人。
        self.publish_bots_and_contacts(&local, views);
        drop(local);
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
                shared: plugin.is_some_and(|plugin| plugin.bot_shared),
                danger: plugin.and_then(|plugin| {
                    (plugin.bot_danger || plugin.bot_danger_can_toggle).then_some(
                        wonton_protocol::DangerModeView {
                            enabled: plugin.bot_danger,
                            can_toggle: plugin.bot_danger_can_toggle,
                            reason: None,
                        },
                    )
                }),
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
}

pub(super) async fn follow_plugins(
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
