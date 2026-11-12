//! Bot 命令菜单：把 ACP 的 `available_commands_update` 写成房间状态事件（ADR-0054）。
//!
//! 每次通知都是整张表，整表替换；表的内容指纹没变就不写。桥自己的 `/wonton` 命令永远排在第一（规整时重名的
//! 后到者被丢掉，Agent 也报 `wonton` 时留下桥的），Agent 断开时回到只有桥自己的表。客户端只认 `state_key`
//! 和发送者相同的事件，所以 `state_key` 是桥自己的 Matrix ID。

use std::future::Future;

use serde_json::Value;
use sha2::{Digest, Sha256};
use wonton_l10n::Lang;
use wonton_matrix::Error as MatrixError;
use wonton_protocol::{BotCommand, BotCommands};

use crate::l10n::text;

/// 写状态事件和提示的目标；实现是 Matrix 房间，测试用内存替身。
pub trait CommandSink {
    fn set_commands(
        &self,
        content: &Value,
    ) -> impl Future<Output = wonton_matrix::Result<()>> + Send;
    fn send_command_text(
        &self,
        text: &str,
    ) -> impl Future<Output = wonton_matrix::Result<()>> + Send;
}

/// 桥自己的命令名，永远排第一。
const OWN_NAME: &str = "wonton";
/// 子命令名（匹配用，不翻译）；参数写法在 `cmd-hint` 的译文里。
const SUBCOMMANDS: [&str; 5] = ["login", "end", "interrupt", "queue", "steer"];

/// `/wonton` 的帮助文字，也让 `/wonton` 出现在菜单里时能照抄。
/// 各条来自 `cmd-hint` 的 “ | ” 分隔。
pub fn help(lang: Lang) -> String {
    let mut help = text(lang, "cmd-help-header", &[]);
    for subcommand in text(lang, "cmd-hint", &[]).split(" | ") {
        help.push_str("\n/wonton ");
        help.push_str(subcommand.trim());
    }
    help
}

/// 是不是要桥回答的 `/wonton` 命令：只有 `/wonton`，或者子命令不属于其他模块处理的那些。
pub fn is_help_request(text: &str) -> bool {
    let Some(rest) = text.trim().strip_prefix("/wonton") else {
        return false;
    };
    if rest.is_empty() {
        return true;
    }
    let Some(rest) = rest.strip_prefix(char::is_whitespace) else {
        return false;
    };
    let subcommand = rest.trim();
    subcommand.is_empty()
        || !SUBCOMMANDS
            .iter()
            .any(|known| subcommand.split(' ').next() == Some(*known))
}

/// 命令表的写入状态：上一张写成功的表的指纹，和这个房间有没有提示过没有权限。
#[derive(Default)]
pub struct Commands {
    fingerprint: Option<[u8; 32]>,
    no_permission_reported: bool,
}

impl Commands {
    pub fn new() -> Self {
        Self::default()
    }

    /// 处理一条 `session/update`。`current_session` 是房间当前的会话，旧会话迟到的通知丢掉。
    pub async fn receive(
        &mut self,
        sink: &impl CommandSink,
        current_session: Option<&str>,
        session_id: &str,
        update: &Value,
        lang: Lang,
    ) {
        if update["sessionUpdate"] != "available_commands_update" {
            return;
        }
        if current_session.is_some_and(|current| current != session_id) {
            return;
        }
        let table = BotCommands::normalize(
            own_command(lang).into_iter().chain(
                update["availableCommands"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(command),
            ),
        );
        self.write(sink, &table, lang).await;
    }

    /// 回到只有桥自己的表：Agent 断开、桥启动时用。
    pub async fn reset(&mut self, sink: &impl CommandSink, lang: Lang) {
        self.write(sink, &BotCommands::normalize(own_command(lang)), lang)
            .await;
    }

    async fn write(&mut self, sink: &impl CommandSink, table: &BotCommands, lang: Lang) {
        let Ok(content) = serde_json::to_value(table) else {
            tracing::warn!("命令菜单编码失败");
            return;
        };
        let Ok(bytes) = serde_json::to_vec(&content) else {
            tracing::warn!("命令菜单编码失败");
            return;
        };
        let fingerprint: [u8; 32] = Sha256::digest(&bytes).into();
        if self.fingerprint == Some(fingerprint) {
            return;
        }
        match sink.set_commands(&content).await {
            Ok(()) => self.fingerprint = Some(fingerprint),
            Err(error) => {
                tracing::warn!(error = %error, "写命令菜单失败");
                if !self.no_permission_reported && is_forbidden(&error) {
                    self.no_permission_reported = true;
                    if let Err(error) = sink
                        .send_command_text(&text(lang, "cmd-no-permission", &[]))
                        .await
                    {
                        tracing::warn!(error = %error, "发送命令菜单提示失败");
                    }
                }
            }
        }
    }
}

fn own_command(lang: Lang) -> [BotCommand; 1] {
    [BotCommand {
        name: OWN_NAME.into(),
        description: text(lang, "cmd-description", &[]),
        hint: Some(text(lang, "cmd-hint", &[])),
    }]
}

/// ACP 的一条命令：`input.hint` 是参数提示，没有时为空。
fn command(value: &Value) -> BotCommand {
    BotCommand {
        name: value["name"].as_str().unwrap_or_default().to_owned(),
        description: value["description"].as_str().unwrap_or_default().to_owned(),
        hint: value["input"]["hint"].as_str().map(ToOwned::to_owned),
    }
}

fn is_forbidden(error: &MatrixError) -> bool {
    matches!(error, MatrixError::Unauthorized { errcode, .. } if errcode == "M_FORBIDDEN")
}

#[cfg(test)]
#[path = "commands_tests.rs"]
mod tests;
