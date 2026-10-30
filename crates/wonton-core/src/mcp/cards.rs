//! 确认卡片的内容（ADR-0016）：标题、说明行、按钮，以及给其他 Matrix 客户端看的纯文本。文字都由核心写好，
//! App 原样渲染。

use serde_json::{Value, json};
use wonton_protocol::{
    Answer, CARD_FIELD, CardChoice, CardLine, ConfirmationCard, ConfirmationState,
};

use crate::plugin::manifest::{Access, LocalizedText, Manifest, Permission};

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

/// 权限的说明，和安装确认页一致。
pub fn permission(permission: &Permission) -> String {
    let access = |access: &Access| match access {
        Access::Read => "读",
        Access::Write => "读写",
    };
    match permission {
        Permission::StorageShare { name, access: a } => format!("{}共享文件夹 {name}", access(a)),
        Permission::StorageHome { access: a } => format!("{}你的私人空间", access(a)),
        Permission::Models => "使用模型网关".to_owned(),
        Permission::Tools => "调用核心和其他插件的工具".to_owned(),
        Permission::MatrixBot => "以 Bot 身份收发消息".to_owned(),
        Permission::Notify => "给你发通知".to_owned(),
        Permission::NetworkEgress => "访问外网和局域网".to_owned(),
        Permission::NetworkHost => "使用宿主的网络（高权限）".to_owned(),
        Permission::DevicesGpu => "使用 GPU（高权限）".to_owned(),
        Permission::Device { path } => format!("访问宿主设备 {path}（高权限）"),
        Permission::DockerSocket => "控制 Docker（最高权限）".to_owned(),
    }
}

/// 安装插件。
pub fn install(requester: &str, manifest: &Manifest) -> Draft {
    let mut lines = vec![
        line(text(&manifest.description)),
        line(format!("版本 {}，来自插件目录", manifest.version)),
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
        ConfirmationState::Rejected => "已拒绝，没有执行。".to_owned(),
        ConfirmationState::Expired => "没有在期限内确认，已过期，没有执行。".to_owned(),
    });
    out
}

/// 卡片的 Matrix 消息内容。
pub fn content(card: &ConfirmationCard) -> Value {
    json!({
        "msgtype": "m.notice",
        "body": body(card),
        CARD_FIELD: card,
    })
}
