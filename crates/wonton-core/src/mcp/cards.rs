//! 确认卡片的内容（ADR-0016）：标题、说明行、按钮，以及给其他 Matrix 客户端看的纯文本。文字都由核心写好，
//! App 原样渲染。

mod permission;

use std::collections::BTreeSet;
use std::time::Duration;

use serde_json::{Value, json};
use wonton_l10n::Arg;
use wonton_protocol::{
    AgentPermissionRequest, Answer, CARD_FIELD, CardChoice, CardLine, ConfirmationCard,
    ConfirmationState, ConfirmationWindow, PermissionRisk, SecretInput, WindowEnd,
};

pub(crate) use permission::set_permission;

use super::file_results::human_size;
use crate::files::share_fs::saving::{SaveTarget, StagedFile};
use crate::l10n;
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
    /// 卡片要主人填的密钥（ADR-0064）；不填密钥的卡片是空的。
    pub secret_inputs: Vec<SecretInput>,
}

impl Draft {
    /// 加上“N 分钟内都允许”，放在拒绝前面。核心自己的工具不走这里（ADR-0024）。
    #[must_use]
    pub fn with_window(mut self, plugin: &str, length: Duration, langs: &Languages) -> Self {
        let secs = length.as_secs();
        let label = if secs.is_multiple_of(60) {
            l10n::text(
                langs,
                "chat-card-window-minutes",
                &[("minutes", Arg::from(secs / 60))],
            )
        } else {
            l10n::text(
                langs,
                "chat-card-window-seconds",
                &[("secs", Arg::from(secs))],
            )
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

pub(crate) fn line(text: impl Into<String>) -> CardLine {
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
fn image_lines(package: &PluginPackage, langs: &Languages) -> Vec<CardLine> {
    match render::dry_run(package, langs) {
        Ok(plan) => plan
            .containers
            .iter()
            .map(|c| {
                line(l10n::text(
                    langs,
                    "chat-card-image",
                    &[
                        ("service", c.service.as_str().into()),
                        ("image", c.image.as_str().into()),
                    ],
                ))
            })
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
    let old = PluginPackage::manifest_of(&record.source, &Languages::default()).ok();
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
        lines.push(emphasized(l10n::text(
            langs,
            "chat-card-ports-added",
            &[("ports", l10n::list(langs, &added_ports).into())],
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
        lines.push(emphasized(l10n::text(
            langs,
            "chat-card-gpu-added",
            &[("services", l10n::list(langs, &added_gpu).into())],
        )));
    }
    lines
}

/// 安装插件。
pub fn install(
    requester: &str,
    manifest: &Manifest,
    secret_inputs: Vec<SecretInput>,
    langs: &Languages,
) -> Draft {
    let mut lines = vec![
        line(text(&manifest.description)),
        line(l10n::text(
            langs,
            "chat-card-version-official",
            &[("version", manifest.version.as_str().into())],
        )),
    ];
    if manifest.permissions.is_empty() {
        lines.push(line(l10n::text(langs, "chat-card-no-permissions", &[])));
    } else {
        lines.push(line(l10n::text(langs, "chat-card-permissions-header", &[])));
        lines.extend(permission_lines(manifest, langs));
    }
    Draft {
        title: install_title(requester, manifest, langs),
        lines,
        choices: choices(
            &l10n::text(langs, "chat-choice-install", &[]),
            &l10n::text(langs, "chat-choice-deny-install", &[]),
        ),
        scope: None,
        plugin_id: Some(manifest.id.to_string()),
        secret_inputs,
    }
}

/// 卡片标题里的应用名：清单里挑好的那种文字。
fn install_title(requester: &str, manifest: &Manifest, langs: &Languages) -> String {
    l10n::text(
        langs,
        "chat-card-install-title",
        &[
            ("requester", requester.into()),
            ("title", text(&manifest.title).into()),
        ],
    )
}

/// Agent 写的插件第一次装（ADR-0033）：未审核、全部权限和镜像，和“以后不再问”的说明。
/// 有很高或最高级权限时这些权限每次更新都问，“不再问”不成立，就不写。
pub fn agent_install(
    requester: &str,
    package: &PluginPackage,
    secret_inputs: Vec<SecretInput>,
    langs: &Languages,
) -> Draft {
    let manifest = &package.manifest;
    let mut lines = vec![
        emphasized(l10n::text(
            langs,
            "chat-card-agent-source",
            &[("requester", requester.into())],
        )),
        line(text(&manifest.description)),
        line(l10n::text(
            langs,
            "chat-card-version",
            &[("version", manifest.version.as_str().into())],
        )),
    ];
    lines.extend(image_lines(package, langs));
    if manifest.permissions.is_empty() {
        lines.push(line(l10n::text(langs, "chat-card-no-permissions", &[])));
    } else {
        lines.push(line(l10n::text(langs, "chat-card-permissions-header", &[])));
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
        lines.push(line(l10n::text(
            langs,
            "chat-card-agent-ask-again",
            &[("requester", requester.into())],
        )));
    }
    Draft {
        title: install_title(requester, manifest, langs),
        lines,
        choices: choices(
            &l10n::text(langs, "chat-choice-install", &[]),
            &l10n::text(langs, "chat-choice-deny-install", &[]),
        ),
        scope: None,
        plugin_id: Some(manifest.id.to_string()),
        secret_inputs,
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
    secret_inputs: Vec<SecretInput>,
    langs: &Languages,
) -> Draft {
    let manifest = &package.manifest;
    let mut lines = vec![
        emphasized(l10n::text(
            langs,
            "chat-card-agent-source",
            &[("requester", requester.into())],
        )),
        line(l10n::text(
            langs,
            "chat-card-update-from",
            &[
                ("from", from.into()),
                ("to", manifest.version.as_str().into()),
            ],
        )),
        line(header),
    ];
    lines.extend_from_slice(added);
    if let Some(notice) = cross_origin_isolation_notice(manifest, langs)
        && !lines.contains(&notice)
    {
        lines.push(notice);
    }
    Draft {
        title: l10n::text(
            langs,
            "chat-card-update-title",
            &[
                ("requester", requester.into()),
                ("title", text(&manifest.title).into()),
            ],
        ),
        lines,
        choices: choices(
            &l10n::text(langs, "chat-choice-update", &[]),
            &l10n::text(langs, "chat-choice-deny-update", &[]),
        ),
        scope: None,
        plugin_id: Some(manifest.id.to_string()),
        secret_inputs,
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
        title: l10n::text(
            langs,
            "chat-card-highest-title",
            &[("title", text(&manifest.title).into())],
        ),
        lines: vec![
            emphasized(notice),
            emphasized(l10n::text(langs, "chat-card-highest-line", &[])),
        ],
        choices: choices(
            &l10n::text(langs, "chat-card-highest-confirm", &[]),
            &l10n::text(langs, "chat-choice-deny-install", &[]),
        ),
        scope: None,
        plugin_id: Some(manifest.id.to_string()),
        secret_inputs: Vec::new(),
    }
}

/// 卸载插件。
pub fn uninstall(requester: &str, title: &str, delete_data: bool, langs: &Languages) -> Draft {
    let data = if delete_data {
        emphasized(l10n::text(langs, "chat-card-uninstall-purge", &[]))
    } else {
        line(l10n::text(langs, "chat-card-uninstall-keep", &[]))
    };
    Draft {
        title: l10n::text(
            langs,
            "chat-card-uninstall-title",
            &[("requester", requester.into()), ("title", title.into())],
        ),
        lines: vec![data],
        choices: choices(
            &l10n::text(langs, "chat-choice-uninstall", &[]),
            &l10n::text(langs, "chat-choice-cancel", &[]),
        ),
        scope: None,
        plugin_id: None,
        secret_inputs: Vec::new(),
    }
}

/// 调用插件里会改数据的工具。参数原样列出，太长时截断。最高级插件的工具写明以 root 执行。
pub fn tool_call(
    requester: &str,
    plugin: &str,
    tool: &str,
    arguments: &Value,
    highest: bool,
    langs: &Languages,
) -> Draft {
    let mut lines = vec![line(l10n::text(langs, "chat-card-tool-data", &[]))];
    if highest {
        lines.push(emphasized(l10n::text(langs, "chat-card-tool-root", &[])));
    }
    lines.push(arguments_line(arguments, langs));
    Draft {
        title: l10n::text(
            langs,
            "chat-card-tool-title",
            &[
                ("requester", requester.into()),
                ("plugin", plugin.into()),
                ("tool", tool.into()),
            ],
        ),
        lines,
        choices: choices(
            &l10n::text(langs, "chat-choice-allow", &[]),
            &l10n::text(langs, "chat-choice-deny", &[]),
        ),
        scope: None,
        plugin_id: None,
        secret_inputs: Vec::new(),
    }
}

fn arguments_line(arguments: &Value, langs: &Languages) -> CardLine {
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
    line(l10n::text(
        langs,
        "chat-card-args",
        &[("args", args.into())],
    ))
}

/// Agent 自己的工具要动手（ADR-0035）：按钮是 Agent 给的选项。没有拒绝类的选项时加一个不带 `choice_id` 的“取消”。
/// Bot 想往主人的主面板钉页（ADR-0052）：第一次要问，同意后不再问。
pub fn pin_main(title: String, note: String, allow: &str, deny: &str) -> Draft {
    Draft {
        title,
        lines: vec![line(note)],
        choices: choices(allow, deny),
        scope: None,
        plugin_id: None,
        secret_inputs: Vec::new(),
    }
}

pub fn agent_permission(
    requester: &str,
    request: &AgentPermissionRequest,
    langs: &Languages,
) -> Draft {
    let mut lines: Vec<_> = request.description.iter().map(line).collect();
    lines.extend(
        request
            .arguments
            .as_ref()
            .map(|args| arguments_line(args, langs)),
    );
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
            label: l10n::text(langs, "chat-choice-cancel", &[]),
            choice_id: None,
        });
    }
    Draft {
        title: l10n::text(
            langs,
            "chat-card-agent-title",
            &[
                ("requester", requester.into()),
                ("title", request.title.as_str().into()),
            ],
        ),
        lines,
        choices: buttons,
        scope: None,
        plugin_id: None,
        secret_inputs: Vec::new(),
    }
}

/// Bot 把文件存进共享文件夹（ADR-0027）。目标就是共享文件夹本身时没有更大的范围可选。
pub fn save_file(
    requester: &str,
    files: &[StagedFile],
    target: &SaveTarget,
    langs: &Languages,
) -> Draft {
    const LISTED: usize = 10;
    let folder = target.folder();
    let mut lines: Vec<CardLine> = files
        .iter()
        .take(LISTED)
        .map(|file| {
            line(format!(
                "· {}",
                l10n::text(
                    langs,
                    "chat-card-save-file",
                    &[
                        ("name", file.name.as_str().into()),
                        ("size", human_size(file.size).into()),
                    ],
                )
            ))
        })
        .collect();
    if files.len() > LISTED {
        lines.push(line(l10n::text(
            langs,
            "chat-card-save-more",
            &[("count", Arg::from((files.len() - LISTED) as u64))],
        )));
    }
    if files.len() > 1 {
        let total: u64 = files.iter().map(|file| file.size).sum();
        lines.push(line(l10n::text(
            langs,
            "chat-card-save-total",
            &[("total", human_size(total).into())],
        )));
    }
    lines.push(emphasized(l10n::text(
        langs,
        "chat-card-save-unreviewed",
        &[],
    )));
    let mut buttons = vec![CardChoice {
        answer: Answer::Approve,
        label: l10n::text(
            langs,
            "chat-card-save-allow",
            &[("folder", folder.as_str().into())],
        ),
        choice_id: None,
    }];
    if !target.dirs.is_empty() {
        buttons.push(CardChoice {
            answer: Answer::ApproveWider,
            label: l10n::text(
                langs,
                "chat-card-save-allow",
                &[("folder", target.share.as_str().into())],
            ),
            choice_id: None,
        });
    }
    buttons.push(CardChoice {
        answer: Answer::Reject,
        label: l10n::text(langs, "chat-choice-deny", &[]),
        choice_id: None,
    });
    Draft {
        title: l10n::text(
            langs,
            "chat-card-save-title",
            &[
                ("requester", requester.into()),
                ("count", Arg::from(files.len() as u64)),
                ("folder", folder.into()),
            ],
        ),
        lines,
        choices: buttons,
        scope: None,
        plugin_id: None,
        secret_inputs: Vec::new(),
    }
}

/// 给其他 Matrix 客户端看的纯文本。
pub fn body(card: &ConfirmationCard, langs: &Languages) -> String {
    let mut out = card.title.clone();
    for line in &card.lines {
        out.push('\n');
        out.push_str(&line.text);
    }
    out.push('\n');
    let message = |id: &str| l10n::text(langs, id, &[]);
    out.push_str(&match &card.state {
        ConfirmationState::Pending => message("chat-card-state-pending"),
        ConfirmationState::Running => message("chat-card-state-running"),
        ConfirmationState::Succeeded { message } => l10n::text(
            langs,
            "chat-card-state-succeeded",
            &[("message", message.as_str().into())],
        ),
        ConfirmationState::Failed { message } => l10n::text(
            langs,
            "chat-card-state-failed",
            &[("message", message.as_str().into())],
        ),
        ConfirmationState::Uncertain { message } => l10n::text(
            langs,
            "chat-card-state-uncertain",
            &[("message", message.as_str().into())],
        ),
        ConfirmationState::Rejected => message("chat-card-state-rejected"),
        ConfirmationState::Expired => message("chat-card-state-expired"),
    });
    if let Some(window) = &card.window {
        out.push('\n');
        out.push_str(&window_text(window, langs));
    }
    out
}

fn window_text(window: &ConfirmationWindow, langs: &Languages) -> String {
    if let Some(reason) = window.ended {
        let count = window.allowed.unwrap_or(0);
        let reason = match reason {
            WindowEnd::Expired => "expired",
            WindowEnd::Stopped => "stopped",
            WindowEnd::CoreRestarted => "restarted",
            WindowEnd::Unknown => "unknown",
        };
        return l10n::text(
            langs,
            "chat-window-ended",
            &[("reason", reason.into()), ("count", Arg::from(count))],
        );
    }
    l10n::text(langs, "chat-window-active", &[])
}

/// 卡片的 Matrix 消息内容。用 `m.text` 的原因见 ADR-0016。
pub fn content(card: &ConfirmationCard, langs: &Languages) -> Value {
    json!({
        "msgtype": "m.text",
        "body": body(card, langs),
        CARD_FIELD: card,
    })
}

#[cfg(test)]
mod tests {
    use wonton_protocol::ConfirmationState;

    use super::*;
    use crate::l10n::CATALOG;

    /// 契约：`wonton-client` 的 `a_waiting_card_wakes_a_killed_app_and_its_edits_stay_quiet` 证明这种消息推得到手机。
    #[test]
    fn a_card_is_a_message_the_default_push_rules_notify() {
        let langs = Languages::default();
        let card = ConfirmationCard {
            secret_inputs: Vec::new(),
            settings: None,
            requester: None,
            origin: None,
            id: "abc".into(),
            state: ConfirmationState::Pending,
            title: "Hermes 想安装 Grafana".into(),
            lines: Vec::new(),
            choices: choices(
                &l10n::text(&langs, "chat-choice-install", &[]),
                &l10n::text(&langs, "chat-choice-deny-install", &[]),
            ),
            expires_at_ms: 1,
            plugin_id: None,
            window: None,
        };
        assert_eq!(content(&card, &langs)["msgtype"], "m.text");
    }

    /// 卡片正文按语言协商：ja 请求给日文，fr-FR 回退英文，两种真的不同。期望值由目录格式化。
    #[test]
    fn the_card_body_follows_the_requested_language() {
        let card = ConfirmationCard {
            secret_inputs: Vec::new(),
            settings: None,
            requester: None,
            origin: None,
            id: "abc".into(),
            state: ConfirmationState::Pending,
            title: "Hermes 想安装 Grafana".into(),
            lines: vec![line("一条说明")],
            choices: Vec::new(),
            expires_at_ms: 1,
            plugin_id: None,
            window: Some(ConfirmationWindow {
                expires_at_ms: 1,
                allowed: Some(2),
                ended: Some(WindowEnd::Expired),
            }),
        };
        for (header, tag) in [("ja", "ja"), ("fr-FR", "en"), ("zh-TW", "zh-Hant")] {
            let langs = Languages::from_header(Some(header));
            let expected = CATALOG.negotiate(&[tag]);
            let body = body(&card, &langs);
            assert!(
                body.contains(&CATALOG.format(expected, "chat-card-state-pending", &[])),
                "{header}: {body}"
            );
            assert!(
                body.contains(&CATALOG.format(
                    expected,
                    "chat-window-ended",
                    &[("reason", "expired".into()), ("count", Arg::from(2u64))]
                )),
                "{header}: {body}"
            );
        }
        let ja = body(&card, &Languages::from_header(Some("ja")));
        let en = body(&card, &Languages::from_header(Some("fr-FR")));
        assert_ne!(ja, en);
    }
}
