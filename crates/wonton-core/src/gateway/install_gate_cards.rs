//! 装、升级门槛（[`super::install_gates`]）发给管理员的卡片：按预览写第一张，选了 NVMe 时再加一张
//! 最高级的第二张（ADR-0036）。卡片前的核对在 [`super::install_gate_checks`]。

use wonton_protocol::{
    Answer, CardChoice, CardLine, InstallPreview, PluginDevice, PluginDeviceKind,
};

use super::Gateway;
use super::install_gate_checks;
use crate::l10n;
use crate::mcp::{Draft, highest_confirm_for, mark_nvme_risk};
use crate::plugin::InstallError;
use crate::plugin::languages::Languages;

/// 预览项是不是 NVMe 盘。不在了的设备项没有容器路径（`target` 为空），所以还要看 ID：
/// [`crate::plugin::device`] 给 NVMe 的 ID 带 `disk:nvme:`。
fn is_nvme(device: &PluginDevice) -> bool {
    device.kind == PluginDeviceKind::Disk
        && (device.target.starts_with("/dev/nvme") || device.id.starts_with("disk:nvme:"))
}

/// 第一张卡片和选了 NVMe 时的第二张。`kept` 是升级时安装记录里还开着的 NVMe 盘名：设备暂时不在、
/// 设备列表读不到时预览认不出它们，沿用记录（`retain` 且没重新选）时照样要第二张卡。
pub(super) fn nvme_admin_cards(
    preview: &InstallPreview,
    selected: &[String],
    (retain, kept): (bool, &[String]),
    langs: &Languages,
) -> Result<(Draft, Option<Draft>), InstallError> {
    let items = preview.devices.as_deref().unwrap_or_default();
    for id in selected {
        if !items.iter().any(|device| &device.id == id) {
            return Err(InstallError::Invalid(l10n::text(
                langs,
                "plugin-install-device-unknown",
                &[("id", id.as_str().into())],
            )));
        }
    }
    let mut first = preview_draft(preview, langs);
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
    if nvme.is_empty() {
        return Ok((first, None));
    }
    let names = || {
        nvme.iter().map(|name| CardLine {
            text: format!("· {name}"),
            emphasis: true,
        })
    };
    first.permissions = preview.permissions.clone();
    mark_nvme_risk(&mut first, langs);
    first.lines.extend(names());
    first.lines.push(CardLine {
        text: l10n::text(langs, "permission-notice-highest-risk", &[]),
        emphasis: true,
    });
    let mut second = highest_confirm_for(
        &preview.title,
        &preview.id,
        first.permissions.clone(),
        langs,
    );
    second.lines.extend(names());
    Ok((first, Some(second)))
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

/// 按来源安装的卡片：一起装的提供方也逐个预览，摘要要和请求一致、能装，选了 NVMe 的并进两张卡里。
pub(super) async fn source_admin_cards(
    gateway: &Gateway,
    request: &wonton_protocol::InstallRequest,
    preview: &InstallPreview,
    langs: &Languages,
) -> Result<(Draft, Option<Draft>), InstallError> {
    let (mut first, mut second) = nvme_admin_cards(preview, &request.devices, (false, &[]), langs)?;
    for provider in &request.install_together {
        let item = gateway
            .plugins
            .preview_source(&provider.request.source, langs)
            .await?;
        if item.id != provider.id {
            return Err(InstallError::Changed(l10n::text(
                langs,
                "plugin-capability-plan-changed",
                &[],
            )));
        }
        install_gate_checks::matches_install_preview(&item, &provider.request.digest, langs)?;
        if provider.request.devices.is_empty() {
            continue;
        }
        let (provider_first, provider_second) =
            nvme_admin_cards(&item, &provider.request.devices, (false, &[]), langs)?;
        if let Some(provider_second) = provider_second {
            first.lines.push(CardLine {
                text: item.title.clone(),
                emphasis: true,
            });
            first.lines.extend(provider_first.lines);
            first.permissions.extend(provider_first.permissions);
            match &mut second {
                Some(second) => {
                    second.lines.extend(provider_second.lines);
                    second.permissions.extend(provider_second.permissions);
                }
                None => second = Some(provider_second),
            }
        }
    }
    Ok((first, second))
}

pub(super) fn preview_draft(preview: &InstallPreview, langs: &Languages) -> Draft {
    let mut lines = vec![CardLine {
        text: l10n::text(
            langs,
            "chat-card-version-official",
            &[("version", preview.version.as_str().into())],
        ),
        emphasis: false,
    }];
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
