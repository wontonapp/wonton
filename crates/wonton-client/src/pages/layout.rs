use std::path::Path;

use serde::{Deserialize, Serialize};
use wonton_protocol::{
    CoreError, LayoutSettings, MAX_PANEL_PAGES, MyAppEntry, MyAppRef, PanelEntry, TabIcon,
    TabLayout, TabRef,
};

use super::{entry_plugin_id, invalid};
use crate::l10n::{CATALOG, language};
use wonton_l10n::l10n;

pub(super) const LAYOUT_FILE: &str = "layout.json";

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct LayoutFile {
    pub(super) server_id: Option<String>,
    pub(super) my_apps: Vec<StoredApp>,
    /// 新装的应用自动加到末尾。
    #[serde(default = "auto_add_default")]
    pub(super) auto_add: bool,
    /// 主聊天页下拉面板里的页面，按显示顺序（ADR-0052）。
    #[serde(default)]
    pub(super) panel: Vec<PanelEntry>,
}

impl LayoutFile {
    /// 账号数据里的一份：我的应用和主面板。本机文件里多出来的缓存（页面标题图标）不进账号数据。
    pub(super) fn layout(&self) -> LayoutSettings {
        LayoutSettings {
            my_apps: self
                .my_apps
                .iter()
                .map(|entry| MyAppEntry {
                    app: entry.app.clone(),
                    title: entry.title.clone(),
                })
                .collect(),
            auto_add: self.auto_add,
            panel: self.panel.clone(),
        }
    }

    /// 这个插件在我的应用里有没有行（插件行或它的页面行都算）。
    pub(super) fn has_plugin(&self, plugin_id: &str) -> bool {
        self.my_apps
            .iter()
            .any(|entry| super::entry_plugin_id(entry) == Some(plugin_id))
    }

    /// 有没有插件级的那一行（它才盖住插件的全部页面）。
    pub(super) fn has_plugin_row(&self, plugin_id: &str) -> bool {
        self.my_apps.iter().any(
            |entry| matches!(&entry.app, MyAppRef::Plugin { plugin_id: id } if id == plugin_id),
        )
    }

    /// 这一行在不在，返回下标。
    pub(super) fn position(&self, app: &MyAppRef) -> Option<usize> {
        self.my_apps.iter().position(|entry| &entry.app == app)
    }
}

fn auto_add_default() -> bool {
    true
}

/// 栈里的一页：哪个页面，和它的标题图标缓存。
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub(super) struct StoredItem {
    pub(super) tab: TabRef,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) page: Option<StoredPage>,
}

/// 我的应用里的一条。
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub(super) struct StoredApp {
    pub(super) app: MyAppRef,
    /// 用户改过的名字。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) title: Option<String>,
    /// 单独加进来的页面行（插件里的某一页、公网网页）：插件不在、列表没读到时靠它显示。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) page: Option<StoredPage>,
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub(super) struct StoredPage {
    pub(super) plugin_title: String,
    pub(super) title: String,
    pub(super) icon: TabIcon,
    pub(super) layout: TabLayout,
}

pub(super) fn read_file(path: &Path) -> Result<LayoutFile, String> {
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
    let file: LayoutFile = serde_json::from_slice(&bytes).map_err(|error| {
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

fn validate_file(file: &LayoutFile) -> Result<(), String> {
    let mut seen = Vec::new();
    for entry in &file.my_apps {
        if seen.contains(&entry.app) {
            return Err(l10n!(CATALOG, language(), "tabs-config-duplicate-page"));
        }
        seen.push(entry.app.clone());
        if let MyAppRef::Page { tab } = &entry.app {
            if matches!(tab, TabRef::Plugin { .. }) && entry.page.is_none() {
                return Err(l10n!(
                    CATALOG,
                    language(),
                    "tabs-config-plugin-page-incomplete"
                ));
            }
            check_url(tab)?;
        }
    }
    validate_panel(&file.panel, MAX_PANEL_PAGES)?;
    Ok(())
}

/// 我的应用和面板共用的配置规矩：不超上限、没有重页。账号数据也用这份。
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

pub(super) fn default_file(server_id: Option<String>) -> LayoutFile {
    LayoutFile {
        server_id,
        my_apps: Vec::new(),
        auto_add: true,
        panel: Vec::new(),
    }
}

pub(super) fn normalize_for_server(file: &mut LayoutFile, server_id: Option<String>) {
    let Some(server_id) = server_id else { return };
    if file.server_id.as_ref().is_some_and(|old| old != &server_id) {
        // 插件和插件里的页面绑在上一台服务器上；公网网页和服务器无关，留着。
        file.my_apps
            .retain(|entry| entry_plugin_id(entry).is_none());
        file.panel.clear();
    }
    file.server_id = Some(server_id);
}

pub(super) fn entry_tab(entry: &StoredApp) -> TabRef {
    match &entry.app {
        MyAppRef::Plugin { .. } => TabRef::Chat,
        MyAppRef::Page { tab } => tab.clone(),
    }
}

/// 收下服务器上的账号数据：整个换成服务器这份，本机的页面标题图标缓存留下。
/// 配置坏了（缺缓存的页面行、重页、非 https）整个不收，返回原因。
pub(super) fn adopt_layout(file: &mut LayoutFile, settings: LayoutSettings) -> Result<(), String> {
    let mut my_apps = Vec::with_capacity(settings.my_apps.len());
    for entry in settings.my_apps {
        let page = match &entry.app {
            MyAppRef::Page { tab } => {
                check_url(tab)?;
                let cached = file
                    .my_apps
                    .iter()
                    .find_map(|old| (old.app == entry.app).then(|| old.page.clone()).flatten());
                if matches!(tab, TabRef::Plugin { .. }) && cached.is_none() {
                    return Err(l10n!(
                        CATALOG,
                        language(),
                        "tabs-config-plugin-page-incomplete"
                    ));
                }
                cached
            }
            MyAppRef::Plugin { .. } => None,
        };
        my_apps.push(StoredApp {
            app: entry.app,
            title: entry.title,
            page,
        });
    }
    validate_panel(&settings.panel, MAX_PANEL_PAGES)?;
    *file = LayoutFile {
        server_id: file.server_id.clone(),
        my_apps,
        auto_add: settings.auto_add,
        panel: settings.panel,
    };
    Ok(())
}

/// 把本机的配置写进账号数据的原文：只换认识的三块，别的键（更新版 App 存的）原样留下。
pub(super) fn overwrite_layout(
    raw: &mut serde_json::Map<String, serde_json::Value>,
    settings: &LayoutSettings,
) {
    for key in ["my_apps", "auto_add", "panel"] {
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

pub(super) fn persist(path: &Path, file: &LayoutFile) -> Result<(), CoreError> {
    let bytes =
        serde_json::to_vec(file).expect("the layout settings only have serializable fields");
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

/// 加一行时的元数据：插件行不用缓存，页面行按插件或网址算标题和图标。
pub(super) fn stored_app(
    app: &MyAppRef,
    plugins: &wonton_protocol::PluginsViewModel,
    title: Option<String>,
) -> Result<StoredApp, CoreError> {
    let page = match app {
        MyAppRef::Plugin { plugin_id } => {
            let Some(installed) = plugins.installed.as_ref() else {
                return Err(CoreError::NotReady {
                    reason: l10n!(CATALOG, language(), "tabs-view-plugin-list-not-ready"),
                });
            };
            let Some(plugin) = installed
                .plugins
                .iter()
                .find(|plugin| plugin.id == *plugin_id)
            else {
                return Err(invalid(&l10n!(
                    CATALOG,
                    language(),
                    "tabs-view-unknown-plugin"
                )));
            };
            if plugin.web_url.is_none() && plugin.bot_user_id.is_none() {
                return Err(invalid(&l10n!(
                    CATALOG,
                    language(),
                    "tabs-view-plugin-not-openable"
                )));
            }
            // 插件行也存一份元数据：列表没读到时侧边栏照常显示这行。
            Some(StoredPage {
                plugin_title: plugin.title.clone(),
                title: plugin.title.clone(),
                icon: TabIcon::Plugin {
                    icon: plugin.icon.clone(),
                },
                layout: TabLayout::Inset,
            })
        }
        MyAppRef::Page { tab } => {
            check_page_tab(tab, plugins)?;
            Some(page_metadata(tab, plugins, title.clone())?)
        }
    };
    Ok(StoredApp {
        app: app.clone(),
        title,
        page,
    })
}

/// 页面行必须是插件页、插件里的某一页或 `https://` 公网网页。
fn check_page_tab(
    tab: &TabRef,
    plugins: &wonton_protocol::PluginsViewModel,
) -> Result<(), CoreError> {
    match tab {
        TabRef::Url { url } => {
            if !is_https_url(url) {
                return Err(invalid(&l10n!(CATALOG, language(), "panel-pin-not-https")));
            }
            Ok(())
        }
        TabRef::Plugin { plugin_id, page } => {
            let plugin = plugins
                .installed
                .as_ref()
                .and_then(|list| list.plugins.iter().find(|plugin| plugin.id == *plugin_id))
                .ok_or_else(|| invalid(&l10n!(CATALOG, language(), "tabs-view-unknown-plugin")))?;
            if plugin.web_url.is_none() {
                return Err(invalid(&l10n!(
                    CATALOG,
                    language(),
                    "tabs-view-plugin-no-page"
                )));
            }
            if let Some(page) = page
                && !plugin.tabs.iter().any(|declared| declared.id == *page)
            {
                return Err(invalid(&l10n!(
                    CATALOG,
                    language(),
                    "tabs-view-plugin-no-page"
                )));
            }
            Ok(())
        }
        TabRef::PluginPath { plugin_id, .. } => {
            let plugin = plugins
                .installed
                .as_ref()
                .and_then(|list| list.plugins.iter().find(|plugin| plugin.id == *plugin_id))
                .ok_or_else(|| invalid(&l10n!(CATALOG, language(), "tabs-view-unknown-plugin")))?;
            if plugin.web_url.is_none() {
                return Err(invalid(&l10n!(
                    CATALOG,
                    language(),
                    "tabs-view-plugin-no-page"
                )));
            }
            // 手填的路径必须留在插件网站里（ADR-0052）。
            super::view::checked_plugin_path(tab, plugin)?;
            Ok(())
        }
        _ => Err(invalid(&l10n!(CATALOG, language(), "tabs-view-not-a-page"))),
    }
}

/// 页面行的缓存元数据。
fn page_metadata(
    tab: &TabRef,
    plugins: &wonton_protocol::PluginsViewModel,
    title: Option<String>,
) -> Result<StoredPage, CoreError> {
    match tab {
        TabRef::Url { url } => {
            let host = url
                .parse::<url::Url>()
                .ok()
                .and_then(|parsed| parsed.host_str().map(str::to_owned))
                .unwrap_or_else(|| url.clone());
            Ok(StoredPage {
                plugin_title: host.clone(),
                title: title.unwrap_or(host),
                icon: TabIcon::Plugin { icon: None },
                layout: TabLayout::Inset,
            })
        }
        TabRef::Plugin { plugin_id, page } => {
            let plugin = plugins
                .installed
                .as_ref()
                .and_then(|list| list.plugins.iter().find(|plugin| plugin.id == *plugin_id))
                .expect("checked by check_page_tab");
            let fresh = super::metadata_for_plugin(plugin, page.as_deref())
                .ok_or_else(|| invalid(&l10n!(CATALOG, language(), "tabs-view-plugin-no-page")))?;
            Ok(StoredPage {
                title: title.unwrap_or(fresh.title),
                ..fresh
            })
        }
        TabRef::PluginPath { plugin_id, path } => {
            let plugin = plugins
                .installed
                .as_ref()
                .and_then(|list| list.plugins.iter().find(|plugin| plugin.id == *plugin_id))
                .expect("checked by check_page_tab");
            Ok(StoredPage {
                plugin_title: plugin.title.clone(),
                title: title.unwrap_or_else(|| super::view::path_title(path)),
                icon: TabIcon::Plugin {
                    icon: plugin.icon.clone(),
                },
                layout: TabLayout::Inset,
            })
        }
        _ => Err(invalid(&l10n!(CATALOG, language(), "tabs-view-not-a-page"))),
    }
}

fn is_https_url(url: &str) -> bool {
    url::Url::parse(url).is_ok_and(|parsed| parsed.scheme() == "https")
}
