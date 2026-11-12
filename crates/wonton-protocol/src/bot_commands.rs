//! Bot 的斜杠命令菜单（ADR-0054）。
//!
//! 每个 Bot 在它所在的每个房间写一个状态事件 [`BOT_COMMANDS_EVENT`]，`state_key` 是 Bot 自己的 Matrix ID，内容是
//! [`BotCommands`]。每次整张表替换，对应 ACP `available_commands_update` 的语义。客户端只认 `state_key` 和发送者
//! 相同、且是房间成员的事件。调用命令就是发一条普通文字消息 “/名字 参数”，群里带上对 Bot 的提及。

use serde::{Deserialize, Serialize};

/// 房间状态事件的类型。
pub const BOT_COMMANDS_EVENT: &str = "app.wonton.bot.commands";
/// 一张表最多几条，多的截掉并记进 [`BotCommands::omitted`]。
pub const MAX_BOT_COMMANDS: usize = 200;
/// 命令名最多几个字符（Unicode 标量）。
pub const MAX_COMMAND_NAME_CHARS: usize = 64;
pub const MAX_COMMAND_DESCRIPTION_CHARS: usize = 256;
pub const MAX_COMMAND_HINT_CHARS: usize = 128;
/// 序列化后的内容上限。Matrix 事件整体不超过 64 KiB，留出信封的余量。
pub const MAX_BOT_COMMANDS_BYTES: usize = 60 * 1024;

/// 一条命令。字段对应 ACP 的 `AvailableCommand`：`hint` 是 `input.hint`，只是参数提示，没有结构化参数。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct BotCommand {
    /// 不带开头的 `/`。可以有冒号、点和中间的空格（`skill:review`、`review branch`），客户端原样补全，不规整。
    pub name: String,
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
}

impl BotCommand {
    /// 客户端读到的条目是否可用：名字非空、不以 `/` 开头、首尾无空白、没有控制字符、不超长。
    pub fn is_valid(&self) -> bool {
        let name = self.name.as_str();
        !name.is_empty()
            && !name.starts_with('/')
            && name.trim() == name
            && !name.chars().any(char::is_control)
            && name.chars().count() <= MAX_COMMAND_NAME_CHARS
    }
}

/// [`BOT_COMMANDS_EVENT`] 的内容。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BotCommands {
    pub commands: Vec<BotCommand>,
    /// 超过条数或大小上限被截掉的条数。客户端在菜单末尾提示“还有 N 条命令没列出”。
    #[serde(default, skip_serializing_if = "is_zero")]
    pub omitted: u32,
}

fn is_zero(value: &u32) -> bool {
    *value == 0
}

impl BotCommands {
    /// 写之前规整：去掉名字开头的 `/` 和首尾空白，丢掉规整后仍不合规的和重名的（先到先得），截短说明和提示，
    /// 再按条数和大小上限截掉末尾的条目。
    pub fn normalize(commands: impl IntoIterator<Item = BotCommand>) -> Self {
        let mut kept: Vec<BotCommand> = Vec::new();
        let mut omitted = 0u32;
        for command in commands {
            let name = command
                .name
                .trim()
                .trim_start_matches('/')
                .trim()
                .to_owned();
            let command = BotCommand {
                name,
                description: truncate(command.description.trim(), MAX_COMMAND_DESCRIPTION_CHARS),
                hint: command
                    .hint
                    .map(|hint| truncate(hint.trim(), MAX_COMMAND_HINT_CHARS))
                    .filter(|hint| !hint.is_empty()),
            };
            if !command.is_valid() || kept.iter().any(|seen| seen.name == command.name) {
                continue;
            }
            if kept.len() == MAX_BOT_COMMANDS {
                omitted += 1;
                continue;
            }
            kept.push(command);
        }
        let mut table = Self {
            commands: kept,
            omitted,
        };
        while table.encoded_len() > MAX_BOT_COMMANDS_BYTES && table.commands.pop().is_some() {
            table.omitted += 1;
        }
        table
    }

    /// 客户端读：只留可用的条目，名字重复的留第一条。
    pub fn usable(&self) -> Vec<BotCommand> {
        let mut usable: Vec<BotCommand> = Vec::new();
        for command in self.commands.iter().filter(|command| command.is_valid()) {
            if !usable.iter().any(|seen| seen.name == command.name) {
                usable.push(command.clone());
            }
        }
        usable
    }

    fn encoded_len(&self) -> usize {
        serde_json::to_vec(self).map_or(usize::MAX, |bytes| bytes.len())
    }
}

fn truncate(text: &str, max_chars: usize) -> String {
    text.chars().take(max_chars).collect()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn command(name: &str) -> BotCommand {
        BotCommand {
            name: name.into(),
            description: "说明".into(),
            hint: None,
        }
    }

    #[test]
    fn the_wire_shape_matches_acp_and_omits_empty_parts() {
        let table = BotCommands::normalize([
            BotCommand {
                name: "/review branch".into(),
                description: "Review a branch".into(),
                hint: Some("branch name".into()),
            },
            command("skill:pdf"),
        ]);
        assert_eq!(
            serde_json::to_value(&table).unwrap(),
            json!({"commands": [
                {"name": "review branch", "description": "Review a branch", "hint": "branch name"},
                {"name": "skill:pdf", "description": "说明"},
            ]})
        );
    }

    #[test]
    fn normalize_drops_invalid_and_duplicate_names_first_wins() {
        let mut first = command("compact");
        first.description = "first".into();
        let table = BotCommands::normalize([
            first,
            command("compact"),
            command("  "),
            command("a\nb"),
            command(&"x".repeat(MAX_COMMAND_NAME_CHARS + 1)),
        ]);
        assert_eq!(table.commands.len(), 1);
        assert_eq!(table.commands[0].description, "first");
        assert_eq!(table.omitted, 0);
    }

    #[test]
    fn overflow_is_counted_not_silently_lost() {
        let table =
            BotCommands::normalize((0..MAX_BOT_COMMANDS + 5).map(|n| command(&format!("c{n}"))));
        assert_eq!(table.commands.len(), MAX_BOT_COMMANDS);
        assert_eq!(table.omitted, 5);

        let long = "长".repeat(MAX_COMMAND_DESCRIPTION_CHARS);
        let big = BotCommands::normalize((0..MAX_BOT_COMMANDS).map(|n| BotCommand {
            name: format!("c{n}"),
            description: long.clone(),
            hint: None,
        }));
        assert!(serde_json::to_vec(&big).unwrap().len() <= MAX_BOT_COMMANDS_BYTES);
        assert_eq!(big.commands.len() + big.omitted as usize, MAX_BOT_COMMANDS);
    }

    #[test]
    fn a_reader_skips_entries_another_writer_got_wrong() {
        let table: BotCommands = serde_json::from_value(json!({
            "commands": [
                {"name": "/bad", "description": ""},
                {"name": "ok", "description": "fine", "future_field": 1},
                {"name": "ok", "description": "dup"},
            ],
            "omitted": 3,
            "future": true,
        }))
        .unwrap();
        let usable = table.usable();
        assert_eq!(usable.len(), 1);
        assert_eq!(usable[0].description, "fine");
        assert_eq!(table.omitted, 3);
    }
}
