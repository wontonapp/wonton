//! 按网址把一页加进我的应用：算出是哪一种页面（ADR-0095）。
use super::*;

impl PagesLink {
    /// 按网址把一页加进我的应用：插件网站里的是插件的页，其他只收 `https://`，Bot 的网页不收。
    /// 手填的网址没写协议时按 `https://` 补上。
    pub(super) fn add_my_app_url(&self, url: &str, title: Option<String>) -> Result<(), CoreError> {
        let (plugins, _, _) = self.inputs();
        let url = url.trim();
        let url = if url.contains("://") {
            url.to_owned()
        } else {
            format!("https://{url}")
        };
        let parsed = Url::parse(&url)
            .ok()
            .filter(|url| !url.cannot_be_a_base())
            .ok_or_else(|| invalid(&l10n!(CATALOG, language(), "page-invalid-url")))?;
        if is_bot_page(&parsed, &plugins, &lock(&self.state)) {
            return Err(invalid(&l10n!(CATALOG, language(), "my-apps-bot-page")));
        }
        let (tab, default_title) = classify_url(&parsed, &plugins)?;
        let app = match &tab {
            TabRef::Plugin { plugin_id, page } if page.is_none() => MyAppRef::Plugin {
                plugin_id: plugin_id.clone(),
            },
            _ => MyAppRef::Page { tab },
        };
        self.add_my_app(app, title.or(default_title), None)
    }
}

/// Bot 的网页从聊天里打开，“我的应用”不收（`stored_app` 也不收 `TabRef::Bot`）。按网址认出来就说这个原因，
/// 免得被当成公网网页或插件页加进去：已核实的 Bot 网页同源的、Bot 网页专用域名下的、
/// 和默认 Bot 共用网站的插件（没明说“没有网页”的）。
fn is_bot_page(url: &Url, plugins: &PluginsViewModel, state: &PagesState) -> bool {
    if bot::owns_url(state, url)
        || url
            .host_str()
            .is_some_and(|host| host.starts_with(wonton_protocol::BOT_WEB_LABEL_PREFIX))
    {
        return true;
    }
    state.bots.iter().any(|view| {
        view.bot.bot == "default"
            && !matches!(
                view.web,
                Some(wonton_protocol::BotWebView::Unavailable {
                    reason: wonton_protocol::BotWebFailure::NoWebui
                })
            )
            && plugins.installed.as_ref().is_some_and(|installed| {
                installed.plugins.iter().any(|plugin| {
                    plugin.id == view.bot.plugin_id
                        && plugin
                            .web_url
                            .as_deref()
                            .and_then(|base| Url::parse(base).ok())
                            .is_some_and(|base| base.origin() == url.origin())
                })
            })
    })
}

/// 装着的插件的网站里就是那个插件的页（路径对上清单就是声明的那页，否则是插件里的某一页）；
/// 不是插件的网站只收 `https://`。返回页面身份和没有用户名字时的默认标题。
fn classify_url(
    parsed: &Url,
    plugins: &PluginsViewModel,
) -> Result<(TabRef, Option<String>), CoreError> {
    if let Some(installed) = &plugins.installed {
        for plugin in &installed.plugins {
            let Some(base) = plugin
                .web_url
                .as_deref()
                .and_then(|base| Url::parse(base).ok())
            else {
                continue;
            };
            if base.scheme() != parsed.scheme()
                || base.host_str() != parsed.host_str()
                || base.port_or_known_default() != parsed.port_or_known_default()
            {
                continue;
            }
            let mut path = parsed.path().to_owned();
            if let Some(query) = parsed.query() {
                path.push('?');
                path.push_str(query);
            }
            if let Some(fragment) = parsed.fragment() {
                path.push('#');
                path.push_str(fragment);
            }
            if path == "/" {
                return Ok((
                    TabRef::Plugin {
                        plugin_id: plugin.id.clone(),
                        page: None,
                    },
                    Some(plugin.title.clone()),
                ));
            }
            if let Some(declared) = plugin.tabs.iter().find(|tab| tab.path == path) {
                return Ok((
                    TabRef::Plugin {
                        plugin_id: plugin.id.clone(),
                        page: Some(declared.id.clone()),
                    },
                    Some(declared.title.clone()),
                ));
            }
            return Ok((
                TabRef::PluginPath {
                    plugin_id: plugin.id.clone(),
                    path,
                },
                Some(plugin.title.clone()),
            ));
        }
    }
    if parsed.scheme() != "https" || parsed.host_str().is_none() {
        return Err(invalid(&l10n!(CATALOG, language(), "page-not-https")));
    }
    Ok((
        TabRef::Url {
            url: parsed.to_string(),
        },
        parsed.host_str().map(str::to_owned),
    ))
}
