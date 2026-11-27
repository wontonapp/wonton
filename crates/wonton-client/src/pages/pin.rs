//! 把正在看的页面加进我的应用：按网址算出是哪一种页面（ADR-0095）。
use super::*;

impl PagesLink {
    /// 把正在看的页面加进我的应用：按网址算出是哪一种页面。
    pub(super) fn pin_current_page(
        &self,
        url: &str,
        title: Option<String>,
    ) -> Result<(), CoreError> {
        let (plugins, _, _) = self.inputs();
        let (tab, default_title) = classify_pin_for_state(url, &plugins, &lock(&self.state))?;
        let app = match &tab {
            TabRef::Plugin { plugin_id, page } if page.is_none() => MyAppRef::Plugin {
                plugin_id: plugin_id.clone(),
            },
            _ => MyAppRef::Page { tab },
        };
        self.add_my_app(app, title.or(default_title), None)
    }
}

/// 正在看的网址是哪一种页面：装着的插件的网站里就是那个插件的页（路径对上清单就是声明的那页，
/// 否则是插件里的某一页）；不是插件的网站只收 `https://`。返回页面身份和没有用户名字时的默认标题。
fn classify_pin_for_state(
    url: &str,
    plugins: &PluginsViewModel,
    state: &PagesState,
) -> Result<(TabRef, Option<String>), CoreError> {
    let shown = state.stack.last().map(|open| &open.tab);
    if let Some(TabRef::Bot { bot, .. }) = shown
        && bot::ready_page(state, bot).is_none()
    {
        return Err(invalid(&l10n!(
            CATALOG,
            language(),
            "tabs-view-plugin-no-page"
        )));
    }
    if let Ok(parsed) = Url::parse(url)
        && let Some(tab) = bot::classify_pin(state, &parsed)
    {
        return Ok(tab);
    }
    if Url::parse(url)
        .ok()
        .and_then(|url| url.host_str().map(str::to_owned))
        .is_some_and(|host| host.starts_with(wonton_protocol::BOT_WEB_LABEL_PREFIX))
    {
        return Err(invalid(&l10n!(
            CATALOG,
            language(),
            "tabs-view-plugin-no-page"
        )));
    }
    if state.bots.iter().any(|view| {
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
                        && plugin.web_url.as_ref().is_some_and(|base| {
                            Url::parse(base)
                                .ok()
                                .zip(Url::parse(url).ok())
                                .is_some_and(|(base, url)| base.origin() == url.origin())
                        })
                })
            })
    }) {
        return Err(invalid(&l10n!(
            CATALOG,
            language(),
            "tabs-view-plugin-no-page"
        )));
    }
    classify_pin(url, plugins)
}

fn classify_pin(
    url: &str,
    plugins: &PluginsViewModel,
) -> Result<(TabRef, Option<String>), CoreError> {
    let parsed = Url::parse(url)
        .ok()
        .filter(|url| !url.cannot_be_a_base())
        .ok_or_else(|| invalid(&l10n!(CATALOG, language(), "page-invalid-url")))?;
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
