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
    #[serde(default = "default_display")]
    pub display: String,
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

fn default_display() -> String {
    "standard".to_owned()
}

pub(super) fn validate(
    tabs: Option<&[ManifestTab]>,
    webui: Option<&WebUi>,
) -> Result<(), ManifestError> {
    let mut problems = Vec::new();
    if let Some(tabs) = tabs {
        if webui.is_none() {
            problems.push("tabs 声明了但没有声明 webui".to_owned());
        }
        if tabs.len() > 4 {
            problems.push("tabs 最多 4 项".to_owned());
        }
        let mut ids = BTreeSet::new();
        for (index, tab) in tabs.iter().enumerate() {
            let field = format!("tabs[{index}]");
            if !is_tab_id(&tab.id) {
                problems.push(format!("{field}.id 只能包含小写字母、数字和连字符"));
            } else if !ids.insert(&tab.id) {
                problems.push(format!("{field}.id `{}` 重复", tab.id));
            }
            for locale in ["zh", "en"] {
                if tab
                    .title
                    .get(locale)
                    .is_none_or(|value| value.trim().is_empty())
                {
                    problems.push(format!("{field}.title 至少要有非空的 {locale}"));
                }
            }
            if tab.icon.as_deref().is_none_or(str::is_empty) {
                problems.push(format!("{field}.icon 必填"));
            }
            if let Some(problem) = validate_path(&format!("{field}.path"), &tab.path) {
                problems.push(problem);
            }
            if !matches!(tab.layout.as_str(), "inset" | "edge-to-edge") {
                problems.push(format!("{field}.layout 不认识 `{}`", tab.layout));
            }
            if !matches!(tab.display.as_str(), "standard" | "fullscreen") {
                problems.push(format!("{field}.display 不认识 `{}`", tab.display));
            }
        }
    }
    if let Some(webui) = webui
        && !matches!(webui.display.as_str(), "standard" | "fullscreen")
    {
        problems.push(format!("webui.display 不认识 `{}`", webui.display));
    }
    if problems.is_empty() {
        Ok(())
    } else {
        Err(ManifestError::Invalid(problems.join("；")))
    }
}

pub(super) fn validate_path(field: &str, path: &str) -> Option<String> {
    if !path.starts_with('/') {
        return Some(format!("{field} 必须以 / 开头"));
    }
    if path.starts_with("//") {
        return Some(format!("{field} 不能以 // 开头"));
    }
    if path.contains('\\') {
        return Some(format!("{field} 不能包含反斜杠"));
    }
    if path.chars().any(char::is_control) {
        return Some(format!("{field} 不能包含控制字符"));
    }
    None
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

pub(super) fn display(value: &str) -> Option<wonton_protocol::TabDisplay> {
    match value {
        "standard" => Some(wonton_protocol::TabDisplay::Standard),
        "fullscreen" => Some(wonton_protocol::TabDisplay::Fullscreen),
        _ => None,
    }
}
