use serde::{Deserialize, Serialize};

use super::LocalizedText;

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
    pub fn icon_refs(&self, index: usize) -> impl Iterator<Item = (String, &str)> + '_ {
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
