use std::path::Path;

use serde::{Deserialize, Serialize};
use wonton_protocol::{
    CoreError, LayoutSettings, MAX_PANEL_PAGES, PanelEntry, TabDisplayChoice, TabDisplaySetting,
    TabIcon, TabLayout, TabRef,
};

use super::{is_https_url, is_plugin_page};
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
    /// 主聊天页下拉面板里的页面，按显示顺序（ADR-0052）。旧文件里没有，按空的算。
    #[serde(default)]
    pub(super) panel: Vec<PanelEntry>,
}

impl TabsFile {
    /// 账号数据里的一份：底栏和主面板。本机文件里多出来的缓存（页面标题图标）不进账号数据。
    pub(super) fn panel_layout(&self) -> LayoutSettings {
        LayoutSettings {
            items: self.items.iter().map(|item| item.tab.clone()).collect(),
            displays: self
                .displays
                .iter()
                .map(|display| TabDisplaySetting {
                    tab: display.tab.clone(),
                    display: display.display,
                })
                .collect(),
            labels: self.labels,
            panel: self.panel.clone(),
        }
    }
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
        check_url(&item.tab)?;
    }
    if !file.items.iter().any(|item| item.tab == TabRef::Chat) {
        return Err(l10n!(CATALOG, language(), "tabs-config-missing-fixed"));
    }
    validate_panel(&file.panel, MAX_PANEL_PAGES)?;
    Ok(())
}

/// 底栏和面板共用的配置规矩：不超上限、没有重页。账号数据也用这份。
pub(super) fn validate_panel(panel: &[PanelEntry], max: usize) -> Result<(), String> {
    if panel.len() > max {
        return Err(l10n!(
            CATALOG,
            language(),
            "panel-config-too-many",
            count = max
        ));
    }
    let mut seen = Vec::new();
    for entry in panel {
        if seen.contains(&entry.tab) {
            return Err(l10n!(CATALOG, language(), "panel-config-duplicate"));
        }
        seen.push(entry.tab.clone());
        check_url(&entry.tab)?;
    }
    Ok(())
}

/// 存下来的公网网页也只收 `https://`（ADR-0052），本机文件和账号数据都查。
fn check_url(tab: &TabRef) -> Result<(), String> {
    if let TabRef::Url { url } = tab
        && !is_https_url(url)
    {
        return Err(l10n!(CATALOG, language(), "panel-pin-not-https"));
    }
    Ok(())
}

pub(super) fn default_file(server_id: Option<String>) -> TabsFile {
    TabsFile {
        server_id: server_id.clone(),
        // 配上了服务器，入口都在侧边栏里，底栏默认只剩“聊天”；没配对时“应用”网格页还是配对页，留在栏上。
        items: if server_id.is_some() {
            vec![StoredItem {
                tab: TabRef::Chat,
                page: None,
            }]
        } else {
            vec![
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
            ]
        },
        displays: Vec::new(),
        labels: true,
        panel: Vec::new(),
    }
}

pub(super) fn normalize_for_server(file: &mut TabsFile, server_id: Option<String>) {
    let Some(server_id) = server_id else { return };
    if file.server_id.as_ref().is_some_and(|old| old != &server_id) {
        // 插件和插件里的页面绑在上一台服务器上；公网网页和服务器无关，留着。
        file.items.retain(|item| !is_plugin_page(&item.tab));
        file.displays.clear();
        file.panel.clear();
    }
    file.server_id = Some(server_id);
}

/// 收下服务器上的账号数据：身份、顺序和选择整个换成服务器这份，本机的页面标题图标缓存留下。
/// 配置坏了（超上限、缺固定的页、重页）整个不收，返回原因。
pub(super) fn adopt_layout(file: &mut TabsFile, settings: LayoutSettings) -> Result<(), String> {
    let mut items = Vec::with_capacity(settings.items.len());
    for tab in settings.items {
        check_url(&tab)?;
        let page = file
            .items
            .iter()
            .find(|item| item.tab == tab)
            .and_then(|item| item.page.clone());
        items.push(StoredItem { tab, page });
    }
    let displays = settings
        .displays
        .into_iter()
        .map(|setting| StoredDisplay {
            tab: setting.tab,
            display: setting.display,
        })
        .collect();
    validate_panel(&settings.panel, MAX_PANEL_PAGES)?;
    let adopted = TabsFile {
        server_id: file.server_id.clone(),
        items,
        displays,
        labels: settings.labels,
        panel: settings.panel,
    };
    if !adopted.items.iter().any(|item| item.tab == TabRef::Chat)
        || !adopted.items.iter().any(|item| item.tab == TabRef::Apps)
    {
        return Err(l10n!(CATALOG, language(), "tabs-config-missing-fixed"));
    }
    if adopted.items.len() > MAX_ITEMS {
        return Err(l10n!(
            CATALOG,
            language(),
            "tabs-config-too-many-items",
            count = MAX_ITEMS
        ));
    }
    *file = adopted;
    Ok(())
}

/// 把本机的配置写进账号数据的原文：只换认识的四块，别的键（更新版 App 存的）原样留下。
pub(super) fn overwrite_layout(
    raw: &mut serde_json::Map<String, serde_json::Value>,
    settings: &LayoutSettings,
) {
    for key in ["items", "displays", "labels", "panel"] {
        raw.remove(key);
    }
    let value =
        serde_json::to_value(settings).expect("the layout settings only have serializable fields");
    if let Some(fields) = value.as_object() {
        for (key, value) in fields {
            raw.insert(key.clone(), value.clone());
        }
    }
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
    matches!(tab, TabRef::Plugin { .. } | TabRef::SharedApp { .. })
}
