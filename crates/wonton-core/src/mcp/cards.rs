//! 确认卡片的内容（ADR-0016）：标题、说明行、按钮，以及给其他 Matrix 客户端看的纯文本。文字都由核心写好，
//! App 原样渲染。

use std::collections::BTreeSet;
use std::time::Duration;

use serde_json::{Value, json};
use wonton_protocol::{
    AgentPermissionRequest, Answer, CARD_FIELD, CardChoice, CardLine, ConfirmationCard,
    ConfirmationState, ConfirmationWindow, PermissionRisk,
};

use super::file_results::human_size;
use crate::files::share_fs::saving::{SaveTarget, StagedFile};
use crate::plugin::languages::Languages;
use crate::plugin::manifest::{LocalizedText, Manifest};
use crate::plugin::privileges::{self, CapabilityRisk};
use crate::plugin::store::InstalledRecord;
use crate::plugin::views::{cross_origin_isolation_notice, permission_line};
use crate::plugin::{PluginPackage, render};

/// 还没编号、没定到期时间的卡片内容。
pub struct Draft {
    pub title: String,
    pub lines: Vec<CardLine>,
    pub choices: Vec<CardChoice>,
    /// 这张卡片能打开放行时段时，时段管哪个插件。
    pub scope: Option<String>,
    /// 卡片关于哪个插件：壳用它画“打开”和“放到底栏”。
    pub plugin_id: Option<String>,
}

impl Draft {
    /// 加上“N 分钟内都允许”，放在拒绝前面。核心自己的工具不走这里（ADR-0024）。
    #[must_use]
    pub fn with_window(mut self, plugin: &str, length: Duration) -> Self {
        let secs = length.as_secs();
        let label = if secs.is_multiple_of(60) {
            format!("{} 分钟内都允许", secs / 60)
        } else {
            format!("{secs} 秒内都允许")
        };
        let at = self.choices.len().saturating_sub(1);
        self.choices.insert(
            at,
            CardChoice {
                answer: Answer::AllowWindow,
                label,
                choice_id: None,
            },
        );
        self.scope = Some(plugin.to_owned());
        self
    }
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
            choice_id: None,
        },
        CardChoice {
            answer: Answer::Reject,
            label: reject.to_owned(),
            choice_id: None,
        },
    ]
}

pub use crate::plugin::views::permission_text as permission;

/// 权限行：说明加风险醒目，有提示的权限（局域网端口、宿主网络、最高级）在下面带一行醒目的提示
/// （ADR-0036 的三级提示，文字由 [`permission_line`] 给）。
fn permission_lines(manifest: &Manifest, langs: &Languages) -> Vec<CardLine> {
    let mut lines = Vec::new();
    for permission in &manifest.permissions {
        let line = permission_line(permission, langs);
        lines.push(CardLine {
            text: format!("· {}", line.text),
            emphasis: line.risk.is_some(),
        });
        if let Some(notice) = line.notice {
            lines.push(emphasized(notice));
        }
    }
    lines
}

/// 服务和镜像：镜像要按摘要固定，主人在卡片上就能看到要跑的是哪些。
fn image_lines(package: &PluginPackage) -> Vec<CardLine> {
    match render::dry_run(package) {
        Ok(plan) => plan
            .containers
            .iter()
            .map(|c| line(format!("服务 {} 用镜像 {}", c.service, c.image)))
            .collect(),
        // 渲染不了的问题由校验和安装报，卡片上不放报错。
        Err(_) => Vec::new(),
    }
}

/// Agent 的更新比装着的宽出来的项（ADR-0033）：权限、局域网端口、GPU 服务和跨源隔离放宽；换镜像不算。
/// 共享文件夹的挂载来自 storage.share 权限，权限行已覆盖。
pub fn added_lines(
    record: &InstalledRecord,
    package: &PluginPackage,
    langs: &Languages,
) -> Vec<CardLine> {
    let manifest = &package.manifest;
    let mut lines = Vec::new();
    for permission in &manifest.permissions {
        if record
            .granted_permissions
            .iter()
            .any(|granted| granted == permission)
        {
            continue;
        }
        let line = permission_line(permission, langs);
        lines.push(CardLine {
            text: format!("· {}", line.text),
            emphasis: line.risk.is_some(),
        });
        if let Some(notice) = line.notice {
            lines.push(emphasized(notice));
        }
    }
    let old = PluginPackage::manifest_of(&record.source).ok();
    if !old
        .as_ref()
        .and_then(|manifest| manifest.webui.as_ref())
        .is_some_and(|webui| webui.cross_origin_isolated)
    {
        lines.extend(cross_origin_isolation_notice(manifest, langs));
    }
    let ports = |m: Option<&Manifest>| {
        m.and_then(|m| m.lan.as_ref())
            .map(|lan| lan.ports.iter().map(|p| p.key()).collect::<BTreeSet<_>>())
            .unwrap_or_default()
    };
    let added_ports: Vec<String> = ports(Some(manifest))
        .difference(&ports(old.as_ref()))
        .cloned()
        .collect();
    if !added_ports.is_empty() {
        lines.push(emphasized(format!(
            "新增局域网端口 {}",
            added_ports.join("、")
        )));
    }
    let services = |m: Option<&Manifest>| {
        m.and_then(|m| m.gpu.as_ref())
            .map(|gpu| gpu.services.iter().cloned().collect::<BTreeSet<_>>())
            .unwrap_or_default()
    };
    let added_gpu: Vec<String> = services(Some(manifest))
        .difference(&services(old.as_ref()))
        .cloned()
        .collect();
    if !added_gpu.is_empty() {
        lines.push(emphasized(format!(
            "新增用显卡的服务 {}",
            added_gpu.join("、")
        )));
    }
    lines
}

/// 安装插件。
pub fn install(requester: &str, manifest: &Manifest, langs: &Languages) -> Draft {
    let mut lines = vec![
        line(text(&manifest.description)),
        line(format!("版本 {}，来自官方索引", manifest.version)),
    ];
    if manifest.permissions.is_empty() {
        lines.push(line("不需要额外的权限"));
    } else {
        lines.push(line("要授予的权限："));
        lines.extend(permission_lines(manifest, langs));
    }
    Draft {
        title: format!("{requester} 想安装 {}", text(&manifest.title)),
        lines,
        choices: choices("安装", "不装"),
        scope: None,
        plugin_id: Some(manifest.id.to_string()),
    }
}

/// Agent 写的插件第一次装（ADR-0033）：未审核、全部权限和镜像，和“以后不再问”的说明。
/// 有很高或最高级权限时这些权限每次更新都问，“不再问”不成立，就不写。
pub fn agent_install(requester: &str, package: &PluginPackage, langs: &Languages) -> Draft {
    let manifest = &package.manifest;
    let mut lines = vec![
        emphasized(format!(
            "来自你的 Agent · {requester}，没有经过审核：代码由 Agent 写，你自己把关"
        )),
        line(text(&manifest.description)),
        line(format!("版本 {}", manifest.version)),
    ];
    lines.extend(image_lines(package));
    if manifest.permissions.is_empty() {
        lines.push(line("不需要额外的权限"));
    } else {
        lines.push(line("要授予的权限："));
        lines.extend(permission_lines(manifest, langs));
    }
    lines.extend(cross_origin_isolation_notice(manifest, langs));
    let always_asks = manifest.permissions.iter().any(|p| {
        matches!(
            privileges::risk(p),
            Some(CapabilityRisk::VeryHigh) | Some(CapabilityRisk::Highest)
        )
    });
    if !always_asks {
        lines.push(line(format!(
            "以后 {requester} 改这个插件不再问，除非要更多权限"
        )));
    }
    Draft {
        title: format!("{requester} 想安装 {}", text(&manifest.title)),
        lines,
        choices: choices("安装", "不装"),
        scope: None,
        plugin_id: Some(manifest.id.to_string()),
    }
}

/// Agent 写的插件的更新：`header` 说这次为什么发卡片（新增了项、申请了很高或最高级、主人要求每次都问），
/// `added` 是要给主人看的行（ADR-0033）。
pub fn agent_update(
    requester: &str,
    package: &PluginPackage,
    header: &str,
    added: &[CardLine],
    from: &str,
    langs: &Languages,
) -> Draft {
    let manifest = &package.manifest;
    let mut lines = vec![
        emphasized(format!(
            "来自你的 Agent · {requester}，没有经过审核：代码由 Agent 写，你自己把关"
        )),
        line(format!("从版本 {from} 更新到 {}", manifest.version)),
        line(header),
    ];
    lines.extend_from_slice(added);
    if let Some(notice) = cross_origin_isolation_notice(manifest, langs)
        && !lines.contains(&notice)
    {
        lines.push(notice);
    }
    Draft {
        title: format!("{requester} 想更新 {}", text(&manifest.title)),
        lines,
        choices: choices("更新", "不更新"),
        scope: None,
        plugin_id: Some(manifest.id.to_string()),
    }
}

/// 很高和最高级的权限行：说明加提示，别的权限不列（ADR-0036 的三级提示）。
pub fn risky_lines(package: &PluginPackage, langs: &Languages) -> Vec<CardLine> {
    package
        .manifest
        .permissions
        .iter()
        .filter_map(|p| {
            let line = permission_line(p, langs);
            matches!(
                line.risk,
                Some(PermissionRisk::VeryHigh | PermissionRisk::Highest)
            )
            .then(|| {
                let mut lines = vec![CardLine {
                    text: format!("· {}", line.text),
                    emphasis: true,
                }];
                if let Some(notice) = line.notice {
                    lines.push(emphasized(notice));
                }
                lines
            })
        })
        .flatten()
        .collect()
}

/// 最高级的安装和更新要多点一步（ADR-0036）：第一张批准后再点这张，两张都同意才执行。
/// 提示文字用 [`permission_line`] 给的，写明代码会以本机 root 身份运行。
pub fn highest_confirm(package: &PluginPackage, langs: &Languages) -> Draft {
    let manifest = &package.manifest;
    let notice = manifest
        .permissions
        .iter()
        .filter_map(|p| {
            let line = permission_line(p, langs);
            (line.risk == Some(PermissionRisk::Highest)).then_some(line.notice)
        })
        .flatten()
        .next()
        .unwrap_or_default();
    Draft {
        title: format!("{} 能控制整台主机", text(&manifest.title)),
        lines: vec![emphasized(notice), emphasized("代码会以本机 root 身份运行")],
        choices: choices("我知道这个插件能控制整台主机", "不装"),
        scope: None,
        plugin_id: Some(manifest.id.to_string()),
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
        scope: None,
        plugin_id: None,
    }
}

/// 调用插件里会改数据的工具。参数原样列出，太长时截断。最高级插件的工具写明以 root 执行。
pub fn tool_call(
    requester: &str,
    plugin: &str,
    tool: &str,
    arguments: &Value,
    highest: bool,
) -> Draft {
    let mut lines = vec![line("这个工具会改动数据")];
    if highest {
        lines.push(emphasized("这个操作以本机 root 身份执行"));
    }
    lines.push(arguments_line(arguments));
    Draft {
        title: format!("{requester} 想用 {plugin} 的 {tool}"),
        lines,
        choices: choices("允许", "拒绝"),
        scope: None,
        plugin_id: None,
    }
}

fn arguments_line(arguments: &Value) -> CardLine {
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
    line(format!("参数：{args}"))
}

/// Agent 自己的工具要动手（ADR-0035）：按钮是 Agent 给的选项。没有拒绝类的选项时加一个不带 `choice_id` 的“取消”。
pub fn agent_permission(requester: &str, request: &AgentPermissionRequest) -> Draft {
    let mut lines: Vec<_> = request.description.iter().map(line).collect();
    lines.extend(request.arguments.as_ref().map(arguments_line));
    let mut buttons: Vec<_> = request
        .options
        .iter()
        .map(|option| CardChoice {
            answer: option.kind.answer(),
            label: option.name.clone(),
            choice_id: Some(option.choice_id.clone()),
        })
        .collect();
    if !buttons.iter().any(|button| button.answer == Answer::Reject) {
        buttons.push(CardChoice {
            answer: Answer::Reject,
            label: "取消".to_owned(),
            choice_id: None,
        });
    }
    Draft {
        title: format!("{requester}：{}", request.title),
        lines,
        choices: buttons,
        scope: None,
        plugin_id: None,
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
        choice_id: None,
    }];
    if !target.dirs.is_empty() {
        buttons.push(CardChoice {
            answer: Answer::ApproveWider,
            label: format!("允许存进 {}", target.share),
            choice_id: None,
        });
    }
    buttons.push(CardChoice {
        answer: Answer::Reject,
        label: "拒绝".to_owned(),
        choice_id: None,
    });
    Draft {
        title: format!("{requester} 想把 {} 个文件存进 {folder}", files.len()),
        lines,
        choices: buttons,
        scope: None,
        plugin_id: None,
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
    if let Some(window) = &card.window {
        out.push('\n');
        out.push_str(&window_text(window));
    }
    out
}

fn window_text(window: &ConfirmationWindow) -> String {
    window
        .ended_note()
        .unwrap_or_else(|| "放行时段生效中，到期前不再询问。".to_owned())
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
            plugin_id: None,
            window: None,
        };
        assert_eq!(content(&card)["msgtype"], "m.text");
    }
}
