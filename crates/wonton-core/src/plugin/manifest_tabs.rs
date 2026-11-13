//! 插件清单里的底栏页面字段与校验。

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use super::manifest::{LocalizedText, ManifestError, WebUi};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManifestTab {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub title: LocalizedText,
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default)]
    pub icon_selected: Option<String>,
    #[serde(default)]
    pub path: String,
    #[serde(default = "default_layout")]
    pub layout: String,
}

impl ManifestTab {
    pub(super) fn icon_refs(&self, index: usize) -> impl Iterator<Item = (String, &str)> + '_ {
        [
            self.icon
                .as_deref()
                .map(|icon| (format!("tabs[{index}].icon"), icon)),
            self.icon_selected
                .as_deref()
                .map(|icon| (format!("tabs[{index}].icon_selected"), icon)),
        ]
        .into_iter()
        .flatten()
    }
}

fn default_layout() -> String {
    "inset".to_owned()
}

pub(super) fn validate(
    tabs: Option<&[ManifestTab]>,
    webui: Option<&WebUi>,
) -> Result<(), ManifestError> {
    let mut problems = Vec::new();
    if let Some(tabs) = tabs {
        if webui.is_none() {
            problems.push("tabs is declared but webui is not".to_owned());
        }
        if tabs.len() > 4 {
            problems.push("tabs allows at most 4 entries".to_owned());
        }
        let mut ids = BTreeSet::new();
        for (index, tab) in tabs.iter().enumerate() {
            let field = format!("tabs[{index}]");
            if !is_tab_id(&tab.id) {
                problems.push(format!(
                    "{field}.id allows only lowercase letters, digits, and hyphens"
                ));
            } else if !ids.insert(&tab.id) {
                problems.push(format!("{field}.id `{}` is duplicated", tab.id));
            }
            for locale in ["zh", "en"] {
                if tab
                    .title
                    .get(locale)
                    .is_none_or(|value| value.trim().is_empty())
                {
                    problems.push(format!("{field}.title needs a non-empty {locale} entry"));
                }
            }
            if tab.icon.as_deref().is_none_or(str::is_empty) {
                problems.push(format!("{field}.icon is required"));
            }
            if let Some(problem) = validate_path(&format!("{field}.path"), &tab.path) {
                problems.push(problem);
            }
            if !matches!(tab.layout.as_str(), "inset" | "edge-to-edge") {
                problems.push(format!(
                    "{field}.layout `{}` is not recognized; use inset or edge-to-edge",
                    tab.layout
                ));
            }
        }
    }
    if problems.is_empty() {
        Ok(())
    } else {
        Err(ManifestError::Invalid(problems.join("; ")))
    }
}

/// 入口路径（`webui.path` 和 `tabs[].path` 共用）的规则：以 / 开头的绝对路径，可以带 `?query`；
/// 不能有 `#`、空白、控制字符；URL 里不允许原样出现的字符（含非 ASCII）必须已经百分号编码——
/// 核心原样拼接、不再编码。规则写在规范（wonton-docs）里，两边保持一致。
pub(crate) fn validate_path(field: &str, path: &str) -> Option<String> {
    if !path.starts_with('/') {
        return Some(format!("{field} must start with /"));
    }
    if path.starts_with("//") {
        return Some(format!("{field} must not start with //"));
    }
    if path.contains('\\') {
        return Some(format!("{field} must not contain a backslash"));
    }
    let chars: Vec<char> = path.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let ch = chars[i];
        if ch == '%' {
            let encoded = chars.get(i + 1).is_some_and(char::is_ascii_hexdigit)
                && chars.get(i + 2).is_some_and(char::is_ascii_hexdigit);
            if !encoded {
                return Some(format!(
                    "{field} has a bare %; write it as full percent-encoding (% followed by two hex digits)"
                ));
            }
            i += 3;
            continue;
        }
        if ch.is_control() {
            return Some(format!(
                "{field} must not contain control characters (U+{:04X})",
                ch as u32
            ));
        }
        if ch.is_whitespace() {
            return Some(format!(
                "{field} must not contain whitespace; write spaces as %20"
            ));
        }
        if ch == '#' {
            return Some(format!("{field} must not contain #; write it as %23"));
        }
        if !ch.is_ascii() || !raw_allowed(ch) {
            return Some(format!(
                "{field} character `{ch}` must be percent-encoded ({}); the home server concatenates paths as-is and does not encode them",
                encoded(ch)
            ));
        }
        i += 1;
    }
    None
}

/// 允许原样出现的字符：RFC 3986 的 path 和 query 里不用转义的字符。
fn raw_allowed(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || "-._~!$&'()*+,;=:@/?".contains(ch)
}

fn encoded(ch: char) -> String {
    let mut buf = [0u8; 4];
    ch.encode_utf8(&mut buf)
        .bytes()
        .map(|byte| format!("%{byte:02X}"))
        .collect()
}

fn is_tab_id(value: &str) -> bool {
    !value.is_empty()
        && value.split('-').all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        })
}

pub(super) fn layout(value: &str) -> Option<wonton_protocol::TabLayout> {
    match value {
        "inset" => Some(wonton_protocol::TabLayout::Inset),
        "edge-to-edge" => Some(wonton_protocol::TabLayout::EdgeToEdge),
        _ => None,
    }
}
