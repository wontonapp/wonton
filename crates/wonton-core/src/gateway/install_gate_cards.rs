//! 装、升级门槛（[`super::install_gates`]）发给管理员的卡片：按预览写第一张；选了 NVMe、生效的权限里
//! 有最高级时再加一张最高级的第二张（ADR-0036）。请求里成员挑的设备、填的设置和密钥、一起装的提供方和
//! 经它用到的高权限能力都写进第一张，管理员批的就是执行要交出去的。卡片前的核对在
//! [`super::install_gate_checks`]。

use wonton_protocol::{
    Answer, CardChoice, CardLine, InstallPreview, PermissionLine, PermissionRisk, PluginDevice,
    PluginDeviceKind, PluginTrust, SettingValue, SettingValues,
};

use super::Gateway;
use super::install_gate_checks;
use crate::l10n;
use crate::mcp::{
    Draft, disclose_connections, disclose_provider, highest_confirm_for, mark_nvme_risk,
};
use crate::plugin::InstallError;
use crate::plugin::languages::Languages;

/// 预览项是不是 NVMe 盘。不在了的设备项没有容器路径（`target` 为空），所以还要看 ID：
/// [`crate::plugin::device`] 给 NVMe 的 ID 带 `disk:nvme:`。
fn is_nvme(device: &PluginDevice) -> bool {
    device.kind == PluginDeviceKind::Disk
        && (device.target.starts_with("/dev/nvme") || device.id.starts_with("disk:nvme:"))
}

/// 生效的权限里有最高级（`docker.socket`、`privileged`、最高级的能力等）。级别和 Agent 卡片
/// 同出 [`crate::plugin::privileges::risk`]；关着的可选项不算。
fn effective_highest(lines: &[PermissionLine]) -> bool {
    lines
        .iter()
        .any(|line| line.risk == Some(PermissionRisk::Highest) && (!line.optional || line.enabled))
}

/// 请求选的设备：每一项都要在预览里，逐个写出名字；NVMe 的名字另给，要第二张卡。
struct Chosen {
    lines: Vec<CardLine>,
    nvme: Vec<String>,
}

/// `retain` 是升级时沿用记录（请求没重新选），`kept` 是记录里还开着的 NVMe 盘名：设备暂时不在、
/// 设备列表读不到时预览认不出它们，沿用时照样要第二张卡。
fn chosen(
    preview: &InstallPreview,
    selected: &[String],
    (retain, kept): (bool, &[String]),
    langs: &Languages,
) -> Result<Chosen, InstallError> {
    let items = preview.devices.as_deref().unwrap_or_default();
    let mut lines = Vec::new();
    for id in selected {
        let Some(device) = items.iter().find(|device| &device.id == id) else {
            return Err(InstallError::Invalid(l10n::text(
                langs,
                "plugin-install-device-unknown",
                &[("id", id.as_str().into())],
            )));
        };
        lines.push(CardLine {
            text: l10n::text(
                langs,
                "gateway-install-card-device",
                &[("device", device.name.as_str().into())],
            ),
            emphasis: true,
        });
    }
    let keeps = retain && selected.is_empty();
    let mut nvme: Vec<String> = items
        .iter()
        .filter(|device| {
            is_nvme(device) && (selected.contains(&device.id) || (keeps && device.chosen))
        })
        .map(|device| device.name.clone())
        .collect();
    if keeps {
        for name in kept {
            if !nvme.contains(name) {
                nvme.push(name.clone());
            }
        }
    }
    Ok(Chosen { lines, nvme })
}

/// 最高级要多的东西：第一张卡要并进的权限和行，以及第二张卡。选了 NVMe 时 `devices.disk` 按最高级标出、
/// 列出盘名；只有最高级权限时第一张卡不变，权限行已经在上面。
struct Highest {
    permissions: Vec<PermissionLine>,
    lines: Vec<CardLine>,
    second: Draft,
}

fn highest(preview: &InstallPreview, nvme: &[String], langs: &Languages) -> Option<Highest> {
    if nvme.is_empty() && !effective_highest(&preview.permissions) {
        return None;
    }
    let names = || {
        nvme.iter().map(|name| CardLine {
            text: format!("· {name}"),
            emphasis: true,
        })
    };
    let mut marked = preview_draft(preview, langs);
    marked.permissions = preview.permissions.clone();
    let mut lines = Vec::new();
    let mut permissions = Vec::new();
    if !nvme.is_empty() {
        mark_nvme_risk(&mut marked, langs);
        permissions.clone_from(&marked.permissions);
        lines.extend(names());
        lines.push(CardLine {
            text: l10n::text(langs, "permission-notice-highest-risk", &[]),
            emphasis: true,
        });
    }
    let mut second = highest_confirm_for(&preview.title, &preview.id, marked.permissions, langs);
    second.lines.extend(names());
    Some(Highest {
        permissions,
        lines,
        second,
    })
}

fn merge_second(second: &mut Option<Draft>, more: Draft) {
    match second {
        Some(second) => {
            second.lines.extend(more.lines);
            second.permissions.extend(more.permissions);
        }
        None => *second = Some(more),
    }
}

/// 第一张卡片和要最高级确认时的第二张。`retained` 见 [`chosen`]。
pub(super) fn admin_cards(
    preview: &InstallPreview,
    selected: &[String],
    retained: (bool, &[String]),
    langs: &Languages,
) -> Result<(Draft, Option<Draft>), InstallError> {
    let chosen = chosen(preview, selected, retained, langs)?;
    let mut first = preview_draft(preview, langs);
    first.lines.extend(chosen.lines);
    let mut second = None;
    if let Some(highest) = highest(preview, &chosen.nvme, langs) {
        first.permissions.extend(highest.permissions);
        first.lines.extend(highest.lines);
        second = Some(highest.second);
    }
    Ok((first, second))
}

fn shown(value: &SettingValue) -> Option<String> {
    match value {
        SettingValue::Flag { value } => Some(value.to_string()),
        SettingValue::Integer { value } => Some(value.to_string()),
        SettingValue::Decimal { value } | SettingValue::Text { value } => Some(value.clone()),
        SettingValue::Texts { value } => Some(value.join(", ")),
        SettingValue::Other => None,
    }
}

/// 请求里填的密钥（只写名字）和设置值：和预览里现有的值一样的不写。设置可能就是“同意许可”这类
/// 没有默认值的必填项，管理员要看到填的是什么。
pub(super) fn filled_lines<'a>(
    preview: &InstallPreview,
    secrets: impl IntoIterator<Item = &'a String>,
    settings: &SettingValues,
    langs: &Languages,
) -> Vec<CardLine> {
    let mut lines: Vec<CardLine> = secrets
        .into_iter()
        .map(|name| {
            let title = preview
                .secret_inputs
                .iter()
                .find(|input| &input.name == name)
                .map_or(name.as_str(), |input| input.title.as_str());
            CardLine {
                text: l10n::text(
                    langs,
                    "gateway-install-card-secret",
                    &[("title", title.into())],
                ),
                emphasis: false,
            }
        })
        .collect();
    let fields = preview
        .settings
        .as_ref()
        .map(|form| form.fields.as_slice())
        .unwrap_or_default();
    let mut names: Vec<&String> = settings.keys().collect();
    names.sort();
    for name in names {
        let value = &settings[name];
        let field = fields.iter().find(|field| &field.name == name);
        if field.is_some_and(|field| field.value.as_ref() == Some(value)) {
            continue;
        }
        let Some(shown) = shown(value) else { continue };
        let title = field.map_or(name.as_str(), |field| field.title.as_str());
        lines.push(CardLine {
            text: l10n::text(
                langs,
                "gateway-install-card-setting",
                &[("title", title.into()), ("value", shown.into())],
            ),
            emphasis: true,
        });
    }
    lines
}

/// 按来源安装的卡片：一起装的提供方要在预览列的提供方里，也逐个预览，摘要和计划要和请求一致、能装。
/// 提供方的标题、权限、选的设备、填的密钥和设置、经它用到的高权限能力都写进第一张卡（同 Agent 卡片的
/// 写法）；它有最高级权限或选了 NVMe 时并进第二张。
pub(super) async fn source_admin_cards(
    gateway: &Gateway,
    request: &wonton_protocol::InstallRequest,
    preview: &InstallPreview,
    langs: &Languages,
) -> Result<(Draft, Option<Draft>), InstallError> {
    let (mut first, mut second) = admin_cards(preview, &request.devices, (false, &[]), langs)?;
    // 执行侧按请求的顺序取第一个能提供的提供方，卡片写的是同一个；同一个 id 请求两次时执行侧才 400，
    // 卡片只写一次。
    let mut installing: Vec<String> = Vec::new();
    for provider in &request.install_together {
        if !installing.contains(&provider.id) {
            installing.push(provider.id.clone());
        }
    }
    first.lines.extend(filled_lines(
        preview,
        request.secrets.keys(),
        &request.settings,
        langs,
    ));
    for id in &installing {
        let provider = request
            .install_together
            .iter()
            .find(|provider| &provider.id == id)
            .expect("刚按请求排的提供方");
        let inner = &provider.request;
        if !preview
            .missing_requires
            .iter()
            .any(|missing| missing.providers.iter().any(|item| item.id == provider.id))
        {
            return Err(InstallError::Invalid(l10n::text(
                langs,
                "plugin-capability-install-selection",
                &[],
            )));
        }
        let item = gateway.plugins.preview_source(&inner.source, langs).await?;
        if item.id != provider.id {
            return Err(InstallError::Changed(l10n::text(
                langs,
                "plugin-capability-plan-changed",
                &[],
            )));
        }
        install_gate_checks::matches_install_preview(&item, &inner.digest, &inner.plan, langs)?;
        let chosen = chosen(&item, &inner.devices, (false, &[]), langs)?;
        disclose_provider(&mut first, &item, inner, langs);
        disclose_connections(&mut first, preview, &item, &installing, langs);
        first.lines.extend(chosen.lines);
        first.lines.extend(filled_lines(
            &item,
            inner.secrets.keys(),
            &inner.settings,
            langs,
        ));
        match highest(&item, &chosen.nvme, langs) {
            Some(highest) => {
                // 选了 NVMe 时用按最高级标过的那份，否则就是预览的权限行。
                if highest.permissions.is_empty() {
                    first.permissions.extend(item.permissions.iter().cloned());
                } else {
                    first.permissions.extend(highest.permissions);
                }
                first.lines.extend(highest.lines);
                merge_second(&mut second, highest.second);
            }
            None => first.permissions.extend(item.permissions.iter().cloned()),
        }
    }
    Ok((first, second))
}

/// 卡片第一行：版本。只有官方索引审核过的写“来自官方索引”；其余来源（上传的包、提供方和第三方索引）
/// 由预览 `notices` 里醒目的未审核提示写一处，版本行不再重复同样的提供方和主机。
fn version_line(preview: &InstallPreview, langs: &Languages) -> CardLine {
    let key = if preview.trust == PluginTrust::Official {
        "chat-card-version-official"
    } else {
        "chat-card-version"
    };
    CardLine {
        text: l10n::text(langs, key, &[("version", preview.version.as_str().into())]),
        emphasis: false,
    }
}

pub(super) fn preview_draft(preview: &InstallPreview, langs: &Languages) -> Draft {
    let mut lines = vec![version_line(preview, langs)];
    lines.extend(preview.permissions.iter().map(|line| CardLine {
        text: format!("· {}", line.text),
        emphasis: line.risk.is_some(),
    }));
    lines.extend(preview.notices.iter().cloned());
    Draft {
        title: preview.title.clone(),
        lines,
        choices: choices(langs),
        scope: None,
        plugin_id: Some(preview.id.clone()),
        secret_inputs: Vec::new(),
        settings: None,
        category: None,
        permissions: Vec::new(),
        devices: Vec::new(),
    }
}

fn choices(langs: &Languages) -> Vec<CardChoice> {
    vec![
        CardChoice {
            answer: Answer::Approve,
            label: l10n::text(langs, "chat-choice-install", &[]),
            choice_id: None,
        },
        CardChoice {
            answer: Answer::Reject,
            label: l10n::text(langs, "chat-choice-deny-install", &[]),
            choice_id: None,
        },
    ]
}
