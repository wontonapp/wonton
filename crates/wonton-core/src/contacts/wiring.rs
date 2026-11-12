//! 把联系人逻辑接到核心的真实组件上：插件管理器里的 Bot 记录、家人记录、推送代理、Matrix 和服务器配对。

use std::path::Path;
use std::sync::{Arc, Weak};
use std::time::Duration;

use wonton_protocol::{BotKeeper, BotRef, ContactRequestPush};

use super::{Contacts, Directory, Effects, LocalBot, Ports, StoreError};
use crate::matrix::Matrix;
use crate::members::Members;
use crate::owner::OwnerStore;
use crate::peers::Peers;
use crate::plugin::manager::PluginManager;
use crate::push::Push;

/// 隔这么久查一遍还在等的人的邀请（ADR-0050：接受了才成立）。对方接受后 Bot 要等到下一次查才理他，一分钟够用。
const CHECK_PEOPLE_INTERVAL: Duration = Duration::from_secs(60);
/// 隔这么久再发一遍还没送到对方核心的联系人事件（[`Contacts::flush_outbox`]）。启动时马上发一次。
const OUTBOX_INTERVAL: Duration = Duration::from_secs(30);

/// 插件管理器里的 Bot 记录。
pub struct PluginDirectory {
    plugins: Arc<PluginManager>,
    members: Arc<Members>,
    owner: Arc<OwnerStore>,
}

impl PluginDirectory {
    pub fn new(plugins: Arc<PluginManager>, members: Arc<Members>, owner: Arc<OwnerStore>) -> Self {
        Self {
            plugins,
            members,
            owner,
        }
    }

    fn local(&self, listing: crate::plugin::contacts::BotListing) -> LocalBot {
        let keeper_display_name = match &listing.keeper {
            BotKeeper::User { username } if username == self.owner.username() => {
                Some(self.owner.owner().display_name)
            }
            BotKeeper::User { username } => {
                self.members.get(username).map(|member| member.display_name)
            }
            BotKeeper::Household | BotKeeper::Other => None,
        };
        LocalBot {
            display_name: self.plugins.bot_display_name(&listing.bot),
            keeper_username: self.plugins.keeper_username(&listing.keeper),
            keeper_display_name,
            bot: listing.bot,
            user_id: listing.user_id,
            keeper: listing.keeper,
        }
    }
}

impl Directory for PluginDirectory {
    fn bot(&self, bot: &BotRef) -> Result<Option<LocalBot>, String> {
        Ok(self
            .plugins
            .bot_listings()?
            .into_iter()
            .find(|listing| &listing.bot == bot)
            .map(|listing| self.local(listing)))
    }

    fn bot_by_user(&self, user_id: &str) -> Result<Option<LocalBot>, String> {
        Ok(self
            .plugins
            .bot_listings()?
            .into_iter()
            .find(|listing| listing.user_id == user_id)
            .map(|listing| self.local(listing)))
    }
}

/// 重建 Bot 的服务、推送给归属人。重建排成一队：同一个 Bot 连着变两次，不能两次重建同时换容器。
pub struct CoreEffects {
    plugins: Arc<PluginManager>,
    push: Option<Arc<Push>>,
    owner_username: String,
    rebuilds: Arc<tokio::sync::Mutex<()>>,
}

impl Effects for CoreEffects {
    fn allowed_users_changed(&self, bot: &BotRef) {
        let plugins = self.plugins.clone();
        let rebuilds = self.rebuilds.clone();
        let bot = bot.clone();
        tokio::spawn(async move {
            let _queue = rebuilds.lock().await;
            match plugins.rebuild_for_contacts(&bot).await {
                Ok(()) => {
                    tracing::info!(plugin = %bot.plugin_id, bot = %bot.bot, "联系人变了，Bot 的名单已更新")
                }
                // 记录是准的：下次重建（启动、升级、再变一次）照记录渲染。
                Err(e) => {
                    tracing::error!(plugin = %bot.plugin_id, bot = %bot.bot, error = %e, "联系人变了，重建 Bot 失败")
                }
            }
        });
    }

    fn notify_request(&self, keeper_username: &str, request: ContactRequestPush) {
        let Some(push) = self.push.clone() else {
            tracing::info!("推送代理没有启用，联系人请求只在列表里");
            return;
        };
        let user = (keeper_username != self.owner_username).then(|| keeper_username.to_owned());
        tokio::spawn(async move {
            push.notify_contact_request(user.as_deref(), request).await;
        });
    }
}

/// 核心启动时接上的东西。
pub struct Wiring {
    pub plugins: Arc<PluginManager>,
    pub members: Arc<Members>,
    pub owner: Arc<OwnerStore>,
    pub push: Option<Arc<Push>>,
    pub matrix: Arc<Matrix>,
    pub peers: Arc<Peers>,
}

/// 读联系人记录、接上通道和插件管理器、起后台任务。
pub fn start(state_dir: &Path, wiring: Wiring) -> Result<Arc<Contacts>, StoreError> {
    let own_server = wiring.matrix.server_name().to_owned();
    let link = super::matrix_link::MatrixLink::open(
        state_dir.join("contact-rooms.json"),
        wiring.matrix.clone(),
        wiring.peers.clone(),
    )?;
    let invites = super::matrix_invites::MatrixInvites::open(
        state_dir.join("contact-sessions.json"),
        wiring.matrix.clone(),
    )?;
    let ports = Ports {
        directory: Arc::new(PluginDirectory {
            plugins: wiring.plugins.clone(),
            members: wiring.members,
            owner: wiring.owner.clone(),
        }),
        link: link.clone(),
        invites: Arc::new(invites),
        effects: Arc::new(CoreEffects {
            plugins: wiring.plugins.clone(),
            push: wiring.push,
            owner_username: wiring.owner.username().to_owned(),
            rebuilds: Arc::default(),
        }),
    };
    let contacts = Contacts::open(state_dir.join("contacts.json"), &own_server, ports)?;
    let source = Arc::downgrade(&contacts);
    wiring
        .plugins
        .set_contact_source(Arc::new(move |bot_user_id: &str| {
            source
                .upgrade()
                .map(|contacts| contacts.allowed_users(bot_user_id))
                .unwrap_or_default()
        }));
    link.start(Arc::downgrade(&contacts));
    spawn_people_check(Arc::downgrade(&contacts));
    spawn_outbox_flush(Arc::downgrade(&contacts));
    Ok(contacts)
}

fn spawn_people_check(contacts: Weak<Contacts>) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(CHECK_PEOPLE_INTERVAL);
        loop {
            tick.tick().await;
            let Some(contacts) = contacts.upgrade() else {
                return;
            };
            contacts.check_people().await;
        }
    });
}

fn spawn_outbox_flush(contacts: Weak<Contacts>) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(OUTBOX_INTERVAL);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tick.tick().await;
            let Some(contacts) = contacts.upgrade() else {
                return;
            };
            contacts.flush_outbox().await;
        }
    });
}
