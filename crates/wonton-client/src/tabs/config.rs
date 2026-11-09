use std::path::Path;

use serde::{Deserialize, Serialize};
use wonton_protocol::{CoreError, TabDisplayChoice, TabIcon, TabLayout, TabRef};

use crate::l10n::{CATALOG, language};
use wonton_l10n::l10n;

pub(super) const TABS_FILE: &str = "tabs.json";
pub(super) const MAX_ITEMS: usize = 5;

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct TabsFile {
    pub(super) server_id: Option<String>,
    pub(super) items: Vec<StoredItem>,
    pub(super) displays: Vec<StoredDisplay>,
    /// 底栏显示文字。旧文件里没有这一项，按默认的显示。
    #[serde(default = "shows_labels")]
    pub(super) labels: bool,
}

fn shows_labels() -> bool {
    true
}

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct StoredItem {
    pub(super) tab: TabRef,
    #[serde(default)]
    pub(super) page: Option<StoredPage>,
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub(super) struct StoredPage {
    pub(super) plugin_title: String,
    pub(super) title: String,
    pub(super) icon: TabIcon,
    pub(super) layout: TabLayout,
}

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct StoredDisplay {
    pub(super) tab: TabRef,
    pub(super) display: TabDisplayChoice,
}

pub(super) fn read_file(path: &Path) -> Result<TabsFile, String> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(default_file(None));
        }
        Err(error) => {
            return Err(l10n!(
                CATALOG,
                language(),
                "tabs-config-read-failed",
                path = path.display().to_string(),
                reason = error.to_string()
            ));
        }
    };
    let file: TabsFile = serde_json::from_slice(&bytes).map_err(|error| {
        l10n!(
            CATALOG,
            language(),
            "tabs-config-corrupt",
            path = path.display().to_string(),
            reason = error.to_string()
        )
    })?;
    validate_file(&file)?;
    Ok(file)
}

fn validate_file(file: &TabsFile) -> Result<(), String> {
    if file.items.len() > MAX_ITEMS {
        return Err(l10n!(
            CATALOG,
            language(),
            "tabs-config-too-many-items",
            count = MAX_ITEMS
        ));
    }
    let mut seen = Vec::new();
    for item in &file.items {
        if seen.contains(&item.tab) {
            return Err(l10n!(CATALOG, language(), "tabs-config-duplicate-page"));
        }
        seen.push(item.tab.clone());
        if is_plugin(&item.tab) && item.page.is_none() {
            return Err(l10n!(
                CATALOG,
                language(),
                "tabs-config-plugin-page-incomplete"
            ));
        }
    }
    if !file.items.iter().any(|item| item.tab == TabRef::Chat)
        || !file.items.iter().any(|item| item.tab == TabRef::Apps)
    {
        return Err(l10n!(CATALOG, language(), "tabs-config-missing-fixed"));
    }
    Ok(())
}

pub(super) fn default_file(server_id: Option<String>) -> TabsFile {
    TabsFile {
        server_id,
        items: vec![
            StoredItem {
                tab: TabRef::Chat,
                page: None,
            },
            StoredItem {
                tab: TabRef::Apps,
                page: None,
            },
            StoredItem {
                tab: TabRef::Settings,
                page: None,
            },
        ],
        displays: Vec::new(),
        labels: true,
    }
}

pub(super) fn normalize_for_server(file: &mut TabsFile, server_id: Option<String>) {
    let Some(server_id) = server_id else { return };
    if file.server_id.as_ref().is_some_and(|old| old != &server_id) {
        file.items.retain(|item| !is_plugin(&item.tab));
        file.displays.clear();
    }
    file.server_id = Some(server_id);
}

pub(super) fn persist(path: &Path, file: &TabsFile) -> Result<(), CoreError> {
    let bytes =
        serde_json::to_vec(file).expect("navigation bar settings only have serializable fields");
    crate::home::write_private(path, &bytes).map_err(|error| CoreError::Storage {
        reason: l10n!(
            CATALOG,
            language(),
            "tabs-config-write-failed",
            path = path.display().to_string(),
            reason = error.to_string()
        ),
    })
}

fn is_plugin(tab: &TabRef) -> bool {
    matches!(tab, TabRef::Plugin { .. })
}
