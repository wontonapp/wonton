//! 清单里的 `matrix_widget`（ADR-0096）：插件网页也是一个 Matrix widget，能在房间的上下文里打开，
//! 经桥说 widget API。核心不认识具体的 widget，只按这里声明和 widget 规范办事。
//!
//! 能力串是 MSC2762 的写法，按事件类型列、不收通配；别的房间（`timeline:`）不给。核心替页面批准哪些，
//! 由驱动按“请求 ∩ 声明 ∩ 核心允许”取交集。

use std::collections::BTreeSet;

use wonton_protocol::CardLine;
use wonton_protocol::{PluginWidget, WidgetCapability, WidgetCapabilityLevel};

use super::languages::Languages;
use super::manifest::{Manifest, ManifestError};
use crate::l10n;

/// 房间顶栏的图标，清单和 App 用同一个枚举：不认识的落到 `Other`。
pub use wonton_protocol::WidgetEntry;

pub use wonton_protocol::manifest::MatrixWidget;

/// 带事件类型的能力前缀。
const TYPED: [&str; 6] = [
    "org.matrix.msc2762.send.event:",
    "org.matrix.msc2762.receive.event:",
    "org.matrix.msc2762.send.state_event:",
    "org.matrix.msc2762.receive.state_event:",
    "org.matrix.msc3819.send.to_device:",
    "org.matrix.msc3819.receive.to_device:",
];

/// 不带事件类型的能力。
const BARE: [&str; 10] = [
    "io.element.requires_client",
    "m.always_on_screen",
    "org.matrix.msc4039.download_file",
    "org.matrix.msc4157.send.delayed_event",
    "org.matrix.msc4157.update_delayed_event",
    "org.matrix.msc4407.receive.sticky_event",
    "org.matrix.msc4407.send.sticky_event",
    "org.matrix.msc4515.rtc_transports",
    "org.matrix.msc4533.rtc_livekit_delegate_delayed_leave",
    "org.matrix.msc4533.rtc_livekit_get_token",
];

/// 读到聊天内容，或者改历史、成员、权限的事件类型：确认页上醒目显示。
const HIGH_TYPES: [&str; 5] = [
    "m.room.encrypted",
    "m.room.member",
    "m.room.message",
    "m.room.power_levels",
    "m.room.redaction",
];

fn invalid(message: String) -> ManifestError {
    ManifestError::Invalid(message)
}

pub(crate) fn validate(manifest: &Manifest) -> Result<(), ManifestError> {
    let Some(widget) = &manifest.matrix_widget else {
        return Ok(());
    };
    let Some(webui) = manifest.webui.as_ref() else {
        return Err(invalid(
            "matrix_widget needs webui: a widget is a page of the plugin".to_owned(),
        ));
    };
    // 声明了跨源隔离的网页在 Android 上交给系统浏览器，那里没有桥，widget 跑不起来。
    if webui.cross_origin_isolated {
        return Err(invalid(
            "matrix_widget and webui.cross_origin_isolated exclude each other: the page opens in the system browser on Android, without the widget bridge"
                .to_owned(),
        ));
    }
    if let Some(problem) = super::manifest_tabs::validate_path("matrix_widget.url", &widget.url) {
        return Err(invalid(problem));
    }
    let mut seen = BTreeSet::new();
    for capability in &widget.capabilities {
        if let Some(problem) = capability_problem(capability) {
            return Err(invalid(problem));
        }
        if !seen.insert(capability.as_str()) {
            return Err(invalid(format!(
                "matrix_widget.capabilities `{capability}` is duplicated"
            )));
        }
    }
    Ok(())
}

/// 能力串不合白名单时，说明它哪里不行。
fn capability_problem(capability: &str) -> Option<String> {
    if capability.is_empty() {
        return Some("matrix_widget.capabilities has an empty entry".to_owned());
    }
    if capability.starts_with("org.matrix.msc2762.timeline:") {
        return Some(format!(
            "matrix_widget.capabilities `{capability}`: the widget only gets the room it was opened in, drop the timeline: entry"
        ));
    }
    if BARE.contains(&capability) {
        return None;
    }
    let Some(rest) = TYPED
        .iter()
        .find_map(|prefix| capability.strip_prefix(prefix))
    else {
        return Some(format!(
            "matrix_widget.capabilities `{capability}` is not recognized"
        ));
    };
    let (event_type, state_key) = match rest.split_once('#') {
        Some((event_type, state_key)) => (event_type, Some(state_key)),
        None => (rest, None),
    };
    // 事件类型的字符集照 Matrix 规范：字母、数字和 `.`、`_`、`-`、`~`，至少两段。
    let event_type_ok = event_type.contains('.')
        && event_type.split('.').all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'~'))
        });
    if !event_type_ok {
        return Some(format!(
            "matrix_widget.capabilities `{capability}`: `{event_type}` is not an event type (no wildcards)"
        ));
    }
    if let Some(state_key) = state_key {
        if state_key.is_empty() {
            return Some(format!(
                "matrix_widget.capabilities `{capability}`: nothing after #"
            ));
        }
        if state_key.contains('*') {
            return Some(format!(
                "matrix_widget.capabilities `{capability}`: state keys take no wildcards"
            ));
        }
    }
    None
}

/// 能力串的级别。str 取事件类型：`#` 前、第一个 `:` 之后。
pub(crate) fn level(capability: &str) -> WidgetCapabilityLevel {
    let Some(rest) = capability.split_once(':').map(|(_, rest)| rest) else {
        return WidgetCapabilityLevel::Normal;
    };
    let event_type = rest
        .split_once('#')
        .map_or(rest, |(event_type, _)| event_type);
    if HIGH_TYPES.contains(&event_type) {
        WidgetCapabilityLevel::High
    } else {
        WidgetCapabilityLevel::Normal
    }
}

pub(crate) fn capabilities(manifest: &Manifest) -> Vec<WidgetCapability> {
    let Some(widget) = &manifest.matrix_widget else {
        return Vec::new();
    };
    lines(&widget.capabilities)
}

pub(crate) fn lines(capabilities: &[String]) -> Vec<WidgetCapability> {
    capabilities
        .iter()
        .map(|capability| WidgetCapability {
            capability: capability.clone(),
            level: level(capability),
            added: false,
        })
        .collect()
}

/// `new` 比 `old` 多出来的能力串。`old` 读不出来时全算多出来的：那个版本的能力没人知道。
pub(crate) fn added(old: Option<&Manifest>, new: &Manifest) -> Vec<String> {
    let Some(widget) = &new.matrix_widget else {
        return Vec::new();
    };
    let current = old
        .and_then(|old| old.matrix_widget.as_ref())
        .map(|widget| widget.capabilities.as_slice())
        .unwrap_or_default();
    let had: BTreeSet<&str> = current.iter().map(String::as_str).collect();
    widget
        .capabilities
        .iter()
        .filter(|capability| !had.contains(capability.as_str()))
        .cloned()
        .collect()
}

/// 确认页、确认卡片和详情页上的风险说明：按级别各一行，高级的醒目。
pub(crate) fn notices(
    capabilities: impl Iterator<Item = WidgetCapabilityLevel>,
    langs: &Languages,
) -> Vec<CardLine> {
    let levels: BTreeSet<WidgetCapabilityLevel> = capabilities.collect();
    let mut lines = Vec::new();
    // 不认识的级别按高级处理。
    if levels.contains(&WidgetCapabilityLevel::High)
        || levels.contains(&WidgetCapabilityLevel::Other)
    {
        lines.push(CardLine {
            text: l10n::text(langs, "plugin-widget-high", &[]),
            emphasis: true,
        });
    }
    if levels.contains(&WidgetCapabilityLevel::Normal) {
        lines.push(CardLine {
            text: l10n::text(langs, "plugin-widget-normal", &[]),
            emphasis: false,
        });
    }
    lines
}

/// 清单声明的全部能力的风险说明。
pub(crate) fn manifest_notices(manifest: &Manifest, langs: &Languages) -> Vec<CardLine> {
    let Some(widget) = &manifest.matrix_widget else {
        return Vec::new();
    };
    notices(widget.capabilities.iter().map(|c| level(c)), langs)
}

/// 已装列表里给 App 的声明：App 在房间里打开 widget 时按它拼地址、批能力。
pub(crate) fn plugin_widget(manifest: &Manifest) -> Option<PluginWidget> {
    manifest.matrix_widget.as_ref().map(|widget| PluginWidget {
        url: widget.url.clone(),
        entry: widget.entry,
        capabilities: widget.capabilities.clone(),
    })
}
