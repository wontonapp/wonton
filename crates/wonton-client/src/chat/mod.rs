//! 聊天：经隧道登录主人的 Matrix 账号，房间列表、时间线、收发消息和端到端加密（ADR-0003、ADR-0005，以及
//! ADR-0009 的“接口：App 登录和使用 Matrix”）。
//!
//! 跟着家里的连接走：本机存着这台服务器的会话就立刻恢复、房间和历史先出来，不等隧道；本地代理就绪（或者换了）再换成
//! 带代理的那次登录。上次登录的是同一台服务器就用存下来的登录状态恢复，不经服务器；否则经隧道向核心要一次性的登录
//! 令牌，用它登录 `matrix.<基础域名>`。所有 Matrix 请求都走本地代理，Tuwunel 只被当成标准的 Matrix 服务器（ADR-0015）。
//!
//! 外部 Matrix 账号的槽位（ADR-0051）不跟着家里的连接走：直连那个账号的 homeserver，用登录时
//! （`matrix_login`）存进槽位的登录状态和存储恢复；登录状态没了只能重新登录那个账号。

mod accessors;
mod agent_run;
pub(crate) mod ask;
mod avatar_files;
mod background;
mod bot_activity;
mod bot_poll;
mod bot_profile;
mod bot_records;
mod bot_typing;
mod bot_web;
mod bots;
mod channels;
mod choices;
mod confirmation;
mod connect;
mod contacts;
mod display_name;
mod draft;
mod edit;
mod edit_actions;
mod follow_home;
mod forward;
mod handoff;
pub(crate) mod inbox;
mod installs;
mod jump;
mod layout;
mod link;
mod link_lifecycle;
mod link_rooms;
mod link_send;
mod link_view;
mod links;
mod login;
pub(crate) mod mentions;
mod menu;
mod onboarding;
mod ops;
mod out_typing;
mod pending;
mod plugin_status;
mod previews;
mod read_mark;
pub(crate) mod recovery;
mod reply;
mod rich;
mod room_commands;
mod room_link;
mod room_rows;
mod room_typing;
mod runs;
mod send;
mod session;
mod side_chat;
mod sign_in;
mod storage;
mod target;
mod timeline;
mod timeline_previews;
mod util;
mod verification;
mod view;

use std::sync::Arc;

use tokio::sync::watch;
use wonton_matrix::{ChatRoom, Error, MatrixSession, OwnedUserId, UserId};
use wonton_protocol::{CORE_LOCALPART, ChatAccount, ChatViewModel, ConfirmationsViewModel};

pub use bots::bot_current_avatar;
#[cfg(not(feature = "test-hooks"))]
pub(crate) use channels::CardSuccess;
#[cfg(feature = "test-hooks")]
pub(crate) use channels::CardSuccess;
pub(crate) use channels::ChatChannels;
pub(crate) use confirmation::fetch_card;
pub(crate) use link::ChatLink;
pub(crate) use links::{LinkTargets, matrix_target};
pub(crate) use login::proxy_url;
pub(crate) use runs::Runs;
#[cfg(feature = "ui-fake")]
pub(crate) use runs::apply_diff;
pub(crate) use storage::{ChatStorage, StoredSession};
pub(crate) use target::TimelineKey;
pub(crate) use timeline::TimelineSource;
pub use timeline::TimelineSubscription;
use util::{invalid, lock};
pub(crate) use view::core_error;
pub(crate) use view::{failure, summary, unread_total};

/// 打开主对话或旁聊的时间线（ADR-0093 §4）。
async fn open_chat_room(
    session: &Arc<MatrixSession>,
    key: &TimelineKey,
) -> Result<ChatRoom, Error> {
    match key.thread() {
        Some(root) => session.watch_thread(&key.room_id, root).await,
        None => session.watch_main(&key.room_id).await,
    }
}

/// 核心账号：和登录的账号 `own_user` 在同一台服务器上。通知房间（核心发插件通知、邮件的私聊）以它认，
/// 别人发的同样结构的内容是普通消息。
pub(crate) fn core_account(own_user: &UserId) -> OwnedUserId {
    format!("@{CORE_LOCALPART}:{}", own_user.server_name())
        .parse()
        .expect("the account's server name with the fixed core localpart is still a valid user ID")
}
