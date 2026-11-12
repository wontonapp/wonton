//! 清单里的 `matrix_widget`（ADR-0096）：插件网页也是一个 Matrix widget，能在房间的上下文里打开，
//! 经桥说 widget API。核心不认识具体的 widget，只按这里声明和 widget 规范办事。
//!
//! 能力串是 MSC2762 的写法，按事件类型列、不收通配；别的房间（`timeline:`）不给。核心替页面批准哪些，
//! 由驱动按“请求 ∩ 声明 ∩ 核心允许”取交集。

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use wonton_protocol::CardLine;
use wonton_protocol::{WidgetCapability, WidgetCapabilityLevel};

use super::languages::Languages;
use super::manifest::{Manifest, ManifestError};
use crate::l10n;

/// 房间顶栏的图标。不认识的落到 [`WidgetEntry::Other`]。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WidgetEntry {
    VideoCall,
    VoiceCall,
    #[default]
    #[serde(other)]
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MatrixWidget {
    /// 网页入口下的路径和查询，如 `/room?widgetId=$matrix_widget_id`。占位符由 widget 规范和核心替换，
    /// 路径规则同 `webui.path`。
    pub url: String,
    #[serde(default)]
    pub entry: WidgetEntry,
    /// 页面要的能力，如 `org.matrix.msc2762.send.state_event:org.matrix.msc3401.call.member`。
    #[serde(default)]
    pub capabilities: Vec<String>,
}

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
const BARE: [&str; 5] = [
    "io.element.requires_client",
    "org.matrix.msc4039.download_file",
    "org.matrix.msc4157.send.delayed_event",
    "org.matrix.msc4157.update_delayed_event",
    "org.matrix.msc4515.rtc_transports",
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
    let event_type_ok = event_type.contains('.')
        && event_type.split('.').all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
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
