//! 从 ViewModel 投影出快照（ADR-0100 决策 2）。只取入口要的：最近会话、默认 Agent、搜索用的人和插件、颜色。

use std::collections::HashSet;

use wonton_protocol::{
    AppearanceViewModel, ChatAccount, ChatFailure, ChatSync, ChatViewModel, HomeFailure,
    HomeStatus, HomeViewModel, OnboardingViewModel, PluginsViewModel, RoomMembership, RoomRow,
    WIDGET_MAX_ROOMS, WIDGET_SNAPSHOT_VERSION, WidgetAccount, WidgetAgent, WidgetContact,
    WidgetPlugin, WidgetRoom, WidgetSnapshot,
};

/// 投影用的各个 ViewModel 此刻的值。
pub(super) struct Views<'a> {
    pub(super) home: &'a HomeViewModel,
    pub(super) chat: &'a ChatViewModel,
    pub(super) onboarding: &'a OnboardingViewModel,
    pub(super) plugins: &'a PluginsViewModel,
    pub(super) appearance: &'a AppearanceViewModel,
    /// 外部 Matrix 账号的槽位（ADR-0051）：没有家里的服务器，不按“没配对”算。
    pub(super) external: bool,
}

/// 此刻该写的快照；状态说不准（正在登录、正在重连、刚恢复会话房间还没到）时为空，文件保持上一次的内容。
/// `updated_at_ms` 留 0，落盘时再填。`previous` 是上一次写下的：插件列表还没取到时沿用它的。
pub(super) fn project(
    views: &Views<'_>,
    previous: Option<&WidgetSnapshot>,
) -> Option<WidgetSnapshot> {
    let account = account(views)?;
    let mut snapshot = empty(account, views.appearance);
    if account != WidgetAccount::SignedIn {
        return Some(snapshot);
    }
    let chat = views.chat;
    let rooms: Vec<&RoomRow> = chat
        .rooms
        .iter()
        .filter(|room| room.membership != RoomMembership::Left)
        .collect();
    snapshot.unread_total = rooms.iter().map(|room| room.unread_count).sum();
    snapshot.rooms = rooms
        .iter()
        .take(WIDGET_MAX_ROOMS)
        .map(|room| WidgetRoom {
            room_id: room.room_id.clone(),
            name: room.name.clone(),
            is_direct: room.is_direct,
            unread_count: room.unread_count,
            preview: room.preview.clone(),
        })
        .collect();
    snapshot.default_agent = views
        .onboarding
        .primary_bot_user_id
        .as_deref()
        .map(|user_id| agent(user_id, chat, &rooms));
    snapshot.contacts = contacts(chat, &rooms);
    snapshot.plugins = match &views.plugins.installed {
        Some(installed) => installed
            .plugins
            .iter()
            .filter(|plugin| plugin.web_url.is_some() || !plugin.tabs.is_empty())
            .map(|plugin| WidgetPlugin {
                plugin_id: plugin.id.clone(),
                title: plugin.title.clone(),
            })
            .collect(),
        // 还没取到不代表没装：沿用上次的，不让搜索里的插件一开 App 就消失。
        None => previous
            .filter(|previous| previous.account == WidgetAccount::SignedIn)
            .map(|previous| previous.plugins.clone())
            .unwrap_or_default(),
    };
    Some(snapshot)
}

/// 换服务器时先写的空快照：上一台的会话和名字不能留在桌面上。
pub(super) fn switching(appearance: &AppearanceViewModel) -> WidgetSnapshot {
    empty(WidgetAccount::Switching, appearance)
}

fn empty(account: WidgetAccount, appearance: &AppearanceViewModel) -> WidgetSnapshot {
    WidgetSnapshot {
        version: WIDGET_SNAPSHOT_VERSION,
        updated_at_ms: 0,
        account,
        unread_total: 0,
        rooms: Vec::new(),
        default_agent: None,
        contacts: Vec::new(),
        plugins: Vec::new(),
        light: appearance.light,
        dark: appearance.dark,
    }
}

fn account(views: &Views<'_>) -> Option<WidgetAccount> {
    if !views.external && views.home.server_id.is_none() {
        return Some(WidgetAccount::NotPaired);
    }
    let revoked = matches!(
        views.chat.account,
        ChatAccount::Failed {
            reason: ChatFailure::Unauthorized,
            ..
        }
    ) || matches!(
        views.home.status,
        HomeStatus::Disconnected {
            reason: HomeFailure::Unauthorized,
            ..
        }
    );
    if revoked {
        return Some(WidgetAccount::SignedOut);
    }
    match views.chat.account {
        // 刚恢复会话、房间列表还没到时说不准，不能写成“没有会话”。
        ChatAccount::SignedIn { .. }
            if views.chat.rooms.is_empty()
                && matches!(views.chat.sync, ChatSync::Idle | ChatSync::Starting) =>
        {
            None
        }
        ChatAccount::SignedIn { .. } => Some(WidgetAccount::SignedIn),
        _ => None,
    }
}

fn agent(user_id: &str, chat: &ChatViewModel, rooms: &[&RoomRow]) -> WidgetAgent {
    let bot = chat.bots.iter().find(|bot| bot.user_id == user_id);
    let direct = rooms
        .iter()
        .find(|room| room.direct_user_id.as_deref() == Some(user_id));
    WidgetAgent {
        user_id: user_id.to_owned(),
        name: bot
            .map(|bot| bot.name.clone())
            .or_else(|| direct.map(|room| room.name.clone()))
            .unwrap_or_else(|| user_id.to_owned()),
        activity: bot.and_then(|bot| bot.activity),
        unread_count: direct.map_or(0, |room| room.unread_count),
    }
}

/// 加入了的私聊的对方，最近聊过的在前，一人一条。只读的通知私聊不算：那不是能聊的人。
fn contacts(chat: &ChatViewModel, rooms: &[&RoomRow]) -> Vec<WidgetContact> {
    let mut seen = HashSet::new();
    rooms
        .iter()
        .filter(|room| room.membership == RoomMembership::Joined && !room.is_read_only)
        .filter_map(|room| {
            let user_id = room.direct_user_id.as_ref()?;
            if !seen.insert(user_id.clone()) {
                return None;
            }
            let bot = chat.bots.iter().find(|bot| &bot.user_id == user_id);
            Some(WidgetContact {
                user_id: user_id.clone(),
                name: bot.map_or_else(|| room.name.clone(), |bot| bot.name.clone()),
                is_bot: bot.is_some(),
                room_id: room.room_id.clone(),
            })
        })
        .collect()
}
