//! 确认卡片的内容（ADR-0016）：标题、说明行、按钮，以及给其他 Matrix 客户端看的纯文本。文字都由核心写好，
//! App 原样渲染。

use serde_json::{Value, json};
use wonton_protocol::{
    Answer, CARD_FIELD, CardChoice, CardLine, ConfirmationCard, ConfirmationState,
};

use super::file_results::human_size;
use crate::files::share_fs::saving::{SaveTarget, StagedFile};
use crate::plugin::manifest::{LocalizedText, Manifest};

/// 还没编号、没定到期时间的卡片内容。
pub struct Draft {
    pub title: String,
    pub lines: Vec<CardLine>,
    pub choices: Vec<CardChoice>,
}

/// 显示用的文字：优先中文。
pub fn text(localized: &LocalizedText) -> String {
    ["zh", "en"]
        .iter()
        .find_map(|lang| localized.get(*lang))
        .or_else(|| localized.values().next())
        .cloned()
        .unwrap_or_default()
}

fn line(text: impl Into<String>) -> CardLine {
    CardLine {
        text: text.into(),
        emphasis: false,
    }
}

fn emphasized(text: impl Into<String>) -> CardLine {
    CardLine {
        text: text.into(),
        emphasis: true,
    }
}

fn choices(approve: &str, reject: &str) -> Vec<CardChoice> {
    vec![
        CardChoice {
            answer: Answer::Approve,
            label: approve.to_owned(),
        },
        CardChoice {
            answer: Answer::Reject,
            label: reject.to_owned(),
        },
    ]
}

pub use crate::plugin::views::permission_text as permission;

/// 安装插件。
pub fn install(requester: &str, manifest: &Manifest) -> Draft {
    let mut lines = vec![
        line(text(&manifest.description)),
        line(format!("版本 {}，来自官方索引", manifest.version)),
    ];
    if manifest.permissions.is_empty() {
        lines.push(line("不需要额外的权限"));
    } else {
        lines.push(line("要授予的权限："));
        lines.extend(manifest.permissions.iter().map(|p| CardLine {
            text: format!("· {}", permission(p)),
            emphasis: p.is_high(),
        }));
    }
    Draft {
        title: format!("{requester} 想安装 {}", text(&manifest.title)),
        lines,
        choices: choices("安装", "不装"),
    }
}

/// 卸载插件。
pub fn uninstall(requester: &str, title: &str, delete_data: bool) -> Draft {
    let data = if delete_data {
        emphasized("并删除它的数据，不可恢复")
    } else {
        line("保留它的数据，以后重装还在")
    };
    Draft {
        title: format!("{requester} 想卸载 {title}"),
        lines: vec![data],
        choices: choices("卸载", "取消"),
    }
}

/// 调用插件里会改数据的工具。参数原样列出，太长时截断。
pub fn tool_call(requester: &str, plugin: &str, tool: &str, arguments: &Value) -> Draft {
    const MAX: usize = 600;
    let mut args = arguments.to_string();
    if args.len() > MAX {
        let cut = (0..=MAX)
            .rev()
            .find(|&i| args.is_char_boundary(i))
            .unwrap_or(0);
        args.truncate(cut);
        args.push('…');
    }
    Draft {
        title: format!("{requester} 想用 {plugin} 的 {tool}"),
        lines: vec![line("这个工具会改动数据"), line(format!("参数：{args}"))],
        choices: choices("允许", "拒绝"),
    }
}

/// Bot 把文件存进共享文件夹（ADR-0027）。目标就是共享文件夹本身时没有更大的范围可选。
pub fn save_file(requester: &str, files: &[StagedFile], target: &SaveTarget) -> Draft {
    const LISTED: usize = 10;
    let folder = target.folder();
    let mut lines: Vec<CardLine> = files
        .iter()
        .take(LISTED)
        .map(|file| line(format!("· {}（{}）", file.name, human_size(file.size))))
        .collect();
    if files.len() > LISTED {
        lines.push(line(format!("还有 {} 个文件", files.len() - LISTED)));
    }
    if files.len() > 1 {
        let total = files.iter().map(|file| file.size).sum();
        lines.push(line(format!("共 {}", human_size(total))));
    }
    lines.push(emphasized("Bot 提供，内容未审核"));
    let mut buttons = vec![CardChoice {
        answer: Answer::Approve,
        label: format!("允许存进 {folder}"),
    }];
    if !target.dirs.is_empty() {
        buttons.push(CardChoice {
            answer: Answer::ApproveWider,
            label: format!("允许存进 {}", target.share),
        });
    }
    buttons.push(CardChoice {
        answer: Answer::Reject,
        label: "拒绝".to_owned(),
    });
    Draft {
        title: format!("{requester} 想把 {} 个文件存进 {folder}", files.len()),
        lines,
        choices: buttons,
    }
}

/// 给其他 Matrix 客户端看的纯文本。
pub fn body(card: &ConfirmationCard) -> String {
    let mut out = card.title.clone();
    for line in &card.lines {
        out.push('\n');
        out.push_str(&line.text);
    }
    out.push('\n');
    out.push_str(&match &card.state {
        ConfirmationState::Pending => "请在 Wonton App 里确认。".to_owned(),
        ConfirmationState::Running => "已确认，正在执行……".to_owned(),
        ConfirmationState::Succeeded { message } => format!("已完成：{message}"),
        ConfirmationState::Failed { message } => format!("没有成功：{message}"),
        ConfirmationState::Uncertain { message } => format!("结果不确定：{message}"),
        ConfirmationState::Rejected => "已拒绝，没有执行。".to_owned(),
        ConfirmationState::Expired => "没有在期限内确认，已过期，没有执行。".to_owned(),
    });
    out
}

/// 卡片的 Matrix 消息内容。用 `m.text` 的原因见 ADR-0016。
pub fn content(card: &ConfirmationCard) -> Value {
    json!({
        "msgtype": "m.text",
        "body": body(card),
        CARD_FIELD: card,
    })
}

#[cfg(test)]
mod tests {
    use wonton_protocol::ConfirmationState;

    use super::*;

    /// 契约：`wonton-client` 的 `a_waiting_card_wakes_a_killed_app_and_its_edits_stay_quiet` 证明这种消息推得到手机。
    #[test]
    fn a_card_is_a_message_the_default_push_rules_notify() {
        let card = ConfirmationCard {
            id: "abc".into(),
            state: ConfirmationState::Pending,
            title: "Hermes 想安装 Grafana".into(),
            lines: Vec::new(),
            choices: choices("安装", "不装"),
            expires_at_ms: 1,
        };
        assert_eq!(content(&card)["msgtype"], "m.text");
    }
}
