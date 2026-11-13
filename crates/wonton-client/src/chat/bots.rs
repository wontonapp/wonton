use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};

use tokio::runtime::Handle;
use tokio::sync::{Notify, watch};
use tokio::time::Instant;
use wonton_matrix::{MatrixSession, UserProfile};
use wonton_protocol::{
    AVATARS_FIELD, AvatarImage, AvatarKind, AvatarMedia, BotActivity, BotActivityFacts, BotRef,
    BotView, ChatForm, ChatViewModel, HomeStatus, InstalledPlugins, MediaHandle, StateAvatar,
    StateAvatars,
};

use super::bot_activity::{Activity, synthesize};
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
    pub refresh_serial: AtomicU64,
    pub local: Mutex<Local>,
}

#[derive(Default)]
pub(super) struct Local {
    pub installed: Option<InstalledPlugins>,
    pub records: HashMap<BotRef, Record>,
    pub report: Option<wonton_protocol::BotActivityReport>,
    pub web_generation: u64,
}

impl Local {
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

    pub fn set_session(self: &Arc<Self>, session: Option<Arc<MatrixSession>>) {
        self.epoch.fetch_add(1, Ordering::SeqCst);
        self.clear_activity();
        self.session.send_replace(session);
        self.changed.notify_one();
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

    fn replace_plugins(self: &Arc<Self>, installed: Option<InstalledPlugins>) {
        let mut local = lock(&self.local);
        if local.installed == installed {
            return;
        }
        let mut records = std::mem::take(&mut local.records);
        let old = self.view.borrow().bots.clone();
        let mut views = Vec::new();
        if let Some(installed) = &installed {
            for plugin in &installed.plugins {
                let Some(user) = plugin.bot_user_id.as_ref().filter(|user| !user.is_empty()) else {
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
                    .and_then(|_| old.iter().find(|bot| bot.bot.plugin_id == plugin.id))
                    .cloned()
                    .unwrap_or_else(|| BotView {
                        chat: Default::default(),
                        user_id: user.clone(),
                        bot: BotRef {
                            plugin_id: plugin.id.clone(),
                            bot: "default".into(),
                        },
                        web: None,
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
                local.records.insert(key, record);
                views.push(bot);
            }
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
        self.update_activity();
        self.refresh_all();
        self.changed.notify_one();
        self.layouts.touch();
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
            }
        });
        for record in local.records.values_mut() {
            record.facts = None;
            record.typing.clear();
            record.activity = synthesize(&record.activity, None, false, Instant::now());
        }
        self.publish_activity(&local);
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

fn handle(url: &str) -> Option<MediaHandle> {
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

    fn view(activity: Option<BotActivity>, states: &[(&str, BotActivity)]) -> BotView {
        BotView {
            chat: Default::default(),
            user_id: "@bot:home".into(),
            bot: BotRef {
                plugin_id: "app.bot".into(),
                bot: "default".into(),
            },
            web: None,
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
}
