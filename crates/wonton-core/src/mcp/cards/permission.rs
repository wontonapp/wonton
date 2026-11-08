//! 开关可选权限的确认卡片（ADR-0057）。

use wonton_protocol::PermissionLine;

use super::{Draft, choices, emphasized, line};
use crate::l10n;
use crate::plugin::languages::Languages;

/// 改一项可选权限：没声明它的级别和提示，`enabled` 打开时要主人点一下。
pub(crate) fn set_permission(
    requester: &str,
    title: &str,
    permission: &PermissionLine,
    enabled: bool,
    highest: bool,
    langs: &Languages,
) -> Draft {
    let mut lines = vec![line(permission.text.clone())];
    if let Some(notice) = &permission.notice {
        lines.push(emphasized(notice.clone()));
    }
    if let Some(off_loss) = &permission.off_loss {
        lines.push(line(l10n::text(
            langs,
            "chat-card-permission-off-loss",
            &[("loss", off_loss.as_str().into())],
        )));
    }
    if highest {
        lines.push(emphasized(l10n::text(
            langs,
            "chat-card-permission-root",
            &[],
        )));
    }
    let (action, cancel) = if enabled {
        (
            l10n::text(langs, "chat-choice-permission-enable", &[]),
            l10n::text(langs, "chat-choice-cancel", &[]),
        )
    } else {
        (
            l10n::text(langs, "chat-choice-permission-disable", &[]),
            l10n::text(langs, "chat-choice-cancel", &[]),
        )
    };
    Draft {
        title: l10n::text(
            langs,
            if enabled {
                "chat-card-permission-enable-title"
            } else {
                "chat-card-permission-disable-title"
            },
            &[
                ("requester", requester.into()),
                ("title", title.into()),
                ("permission", permission.text.as_str().into()),
            ],
        ),
        lines,
        choices: choices(&action, &cancel),
        scope: None,
        plugin_id: None,
        secret_inputs: Vec::new(),
    }
}
