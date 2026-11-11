//! 聊天显示方式的合成（ADR-0060）：和插件 Bot 的私聊按用户对这个 Bot 的选择、插件的默认、全局默认依次取，
//! 其余房间按全局默认。壳照 [`ChatLayout`] 画，不自己合并。

use std::sync::{Arc, Mutex};

use futures_util::{Stream, StreamExt};
use serde_json::json;
use tokio::sync::watch;
use tokio::task::JoinHandle;
use wonton_matrix::{ChatLayoutContent, Error, MatrixSession, OwnedUserId, SyncStatus};
use wonton_protocol::{
    BotChatSettings, ChatFailure, ChatForm, ChatLayout, ChatLayoutChoice, ChatLayoutSettings,
    ChatLayoutTarget, ChatProblem, ChatTextScale, CodeOverflow, CoreError, PluginChat,
};

use super::view;
use super::{ChatLink, failure, invalid, lock};
use crate::l10n::{CATALOG, language};
use wonton_l10n::l10n;

/// 账号数据里的一份选择，每次变化都通知订阅者。
pub(crate) struct Layouts {
    settings: watch::Sender<Arc<ChatLayoutSettings>>,
    /// 这份选择属于哪台服务器。
    server: Mutex<Option<String>>,
}

impl Layouts {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            settings: watch::Sender::new(Arc::default()),
            server: Mutex::default(),
        })
    }

    pub fn current(&self) -> Arc<ChatLayoutSettings> {
        self.settings.borrow().clone()
    }

    pub fn subscribe(&self) -> watch::Receiver<Arc<ChatLayoutSettings>> {
        self.settings.subscribe()
    }

    /// 换成服务器上的这一份。解析不了的字段按没设处理，认不出的取值留在 `Other`。
    pub fn replace(&self, content: ChatLayoutContent) -> Arc<ChatLayoutSettings> {
        let settings: Arc<ChatLayoutSettings> = Arc::new(
            serde_json::from_value(serde_json::Value::Object(content.0)).unwrap_or_default(),
        );
        self.settings.send_replace(settings.clone());
        settings
    }

    /// 内容没变、但别的输入变了（装了插件）：让订阅者重算。
    pub fn touch(&self) {
        let settings = self.current();
        self.settings.send_replace(settings);
    }

    /// 账号数据换到 `server_id` 这台服务器；不是原来那台时清空，返回清没清。
    pub fn adopt(&self, server_id: &str) -> bool {
        let mut server = lock(&self.server);
        if server.as_deref() == Some(server_id) {
            return false;
        }
        *server = Some(server_id.to_owned());
        self.settings.send_replace(Arc::default());
        true
    }
}

/// 一个房间的显示方式。`bot` 是私聊对方的 Matrix 用户 ID 和它所属插件声明的形态，不是插件 Bot 的私聊时为 `None`。
pub(crate) fn compose(
    settings: &ChatLayoutSettings,
    bot: Option<(&str, Option<ChatForm>)>,
) -> ChatLayout {
    let choice = bot.and_then(|(user, _)| settings.bots.get(user));
    let plugin = bot.and_then(|(_, form)| known(form, ChatForm::Other));
    ChatLayout {
        form: choice
            .and_then(|choice| known(choice.form, ChatForm::Other))
            .or(plugin)
            .or(known(settings.default.form, ChatForm::Other))
            .unwrap_or_default(),
        text_scale: choice
            .and_then(|choice| known(choice.text_scale, ChatTextScale::Other))
            .or(known(settings.default.text_scale, ChatTextScale::Other))
            .unwrap_or_default(),
        code: choice
            .and_then(|choice| known(choice.code, CodeOverflow::Other))
            .or(known(settings.default.code, CodeOverflow::Other))
            .unwrap_or_default(),
    }
}

/// Bot 资料页的“显示”一节：默认是插件默认合进全局默认的结果，选择是用户对这个 Bot 改过的。
pub(crate) fn bot_settings(
    settings: &ChatLayoutSettings,
    plugin: Option<&PluginChat>,
    user: &str,
) -> BotChatSettings {
    let plugin = plugin.and_then(|plugin| known(Some(plugin.form), ChatForm::Other));
    BotChatSettings {
        default: ChatLayoutChoice {
            form: Some(
                plugin
                    .or(known(settings.default.form, ChatForm::Other))
                    .unwrap_or_default(),
            ),
            text_scale: Some(
                known(settings.default.text_scale, ChatTextScale::Other).unwrap_or_default(),
            ),
            code: Some(known(settings.default.code, CodeOverflow::Other).unwrap_or_default()),
        },
        choice: settings.bots.get(user).cloned().unwrap_or_default(),
    }
}

/// 这个 App 不认识的取值当作没设。
fn known<T: Copy + PartialEq>(value: Option<T>, unknown: T) -> Option<T> {
    value.filter(|value| *value != unknown)
}

/// 按原始 JSON 只改目标那一处：`choice` 里写了的项写进去，为空的项删掉（回到默认值），其余字段原样保留。
pub(crate) fn apply_layout(
    raw: &mut serde_json::Map<String, serde_json::Value>,
    target: &ChatLayoutTarget,
    choice: &ChatLayoutChoice,
) {
    let mut slot = match target {
        ChatLayoutTarget::Default => take(raw, "default"),
        ChatLayoutTarget::Bot { user_id } => {
            let mut bots = take(raw, "bots");
            let mut slot = take(&mut bots, user_id);
            set_choice(&mut slot, choice);
            if !slot.is_empty() {
                bots.insert(user_id.clone(), slot.into());
            }
            if !bots.is_empty() {
                raw.insert("bots".to_owned(), bots.into());
            }
            return;
        }
    };
    set_choice(&mut slot, choice);
    if !slot.is_empty() {
        raw.insert("default".to_owned(), slot.into());
    }
}

fn take(
    raw: &mut serde_json::Map<String, serde_json::Value>,
    key: &str,
) -> serde_json::Map<String, serde_json::Value> {
    match raw.remove(key) {
        Some(serde_json::Value::Object(map)) => map,
        _ => serde_json::Map::new(),
    }
}

fn set_choice(slot: &mut serde_json::Map<String, serde_json::Value>, choice: &ChatLayoutChoice) {
    set_value(slot, "form", choice.form, ChatForm::Other);
    set_value(slot, "text_scale", choice.text_scale, ChatTextScale::Other);
    set_value(slot, "code", choice.code, CodeOverflow::Other);
}

fn set_value<T: serde::Serialize + PartialEq>(
    slot: &mut serde_json::Map<String, serde_json::Value>,
    key: &str,
    value: Option<T>,
    unknown: T,
) {
    match value {
        Some(value) if value == unknown => {}
        Some(value) => {
            let value = json!(value);
            match slot.get_mut(key) {
                Some(current) => *current = value,
                None => {
                    slot.insert(key.to_owned(), value);
                }
            }
        }
        None => {
            slot.remove(key);
        }
    }
}

impl ChatLink {
    /// 改一份显示方式：写进账号数据，成功以后以服务器回来的内容为准。失败时说明原因，显示不变。
    pub(super) fn set_chat_layout(
        self: &Arc<Self>,
        target: ChatLayoutTarget,
        choice: ChatLayoutChoice,
    ) -> Result<(), CoreError> {
        let session = self.current()?;
        // 账号数据按 Matrix 用户 ID 记，格式不对的值写进去就再也对不上这个 Bot。
        let target = match target {
            ChatLayoutTarget::Default => ChatLayoutTarget::Default,
            ChatLayoutTarget::Bot { user_id } => ChatLayoutTarget::Bot {
                user_id: user_id
                    .parse::<OwnedUserId>()
                    .map_err(|_| {
                        invalid(&l10n!(CATALOG, language(), "chat-layout-invalid-bot-id"))
                    })?
                    .to_string(),
            },
        };
        let chat = self.clone();
        self.runtime.spawn(async move {
            let written = session
                .update_chat_layout(|raw| apply_layout(raw, &target, &choice))
                .await;
            match written {
                Ok(content) => chat.apply_layout(content),
                Err(error) => chat.show_layout_problem(&error),
            }
        });
        Ok(())
    }

    /// 服务器上的选择变了（这里写的，或者另一台设备写的）：本地跟着换，房间和 Bot 资料页重算。
    pub(super) fn apply_layout(self: &Arc<Self>, content: ChatLayoutContent) {
        let settings = self.layouts.replace(content);
        self.view
            .send_modify(|view| view.chat_layout_default = settings.default.clone());
        self.bots.refresh_chat();
    }

    /// 取用显示方式没成功：说明原因，界面照旧（失败即停，不静默）。
    pub(super) fn show_layout_problem(&self, error: &Error) {
        let reason = failure(error);
        tracing::info!("chat layout problem: {error}");
        let message = if reason == ChatFailure::TransportUnavailable {
            view::not_connected()
        } else {
            l10n!(
                CATALOG,
                language(),
                "chat-layout-change-failed",
                reason = error.to_string()
            )
        };
        self.view
            .send_modify(|view| view.problem = Some(ChatProblem { reason, message }));
    }
}

/// 跟着账号数据：每次变化换成本地的一份。
///
/// 取不到时说明原因，等同步再次跑起来（连接断过又回来）再跟一次：这段时间里取也取不到，不加定时重试。
pub(super) fn follow(chat: &Arc<ChatLink>, session: &Arc<MatrixSession>) -> Option<JoinHandle<()>> {
    let session = session.clone();
    let runtime = chat.runtime.clone();
    let chat = Arc::downgrade(chat);
    Some(runtime.spawn(async move {
        let mut statuses = std::pin::pin!(session.sync_status());
        loop {
            let Some(chat) = chat.upgrade() else { return };
            let changes = match session.chat_layout_changes().await {
                Ok(changes) => changes,
                Err(error) => {
                    chat.show_layout_problem(&error);
                    if !synced_again(&mut statuses).await {
                        return;
                    }
                    continue;
                }
            };
            // 取到了：刚才那条原因不再成立，显示方式回来了。
            chat.view
                .send_if_modified(|view| view.problem.take().is_some());
            let mut changes = std::pin::pin!(changes);
            while let Some(content) = changes.next().await {
                chat.apply_layout(content);
            }
            return;
        }
    }))
}

/// 等同步停下过再重新跑起来：断过线的连接才值得再取一次，不加定时重试。
async fn synced_again(statuses: &mut (impl Stream<Item = SyncStatus> + Unpin)) -> bool {
    let mut stopped = false;
    while let Some(status) = statuses.next().await {
        match status {
            SyncStatus::Running if stopped => return true,
            status => stopped |= sync_stopped(&status),
        }
    }
    false
}

fn sync_stopped(status: &SyncStatus) -> bool {
    matches!(status, SyncStatus::Idle | SyncStatus::Failed(_))
}
