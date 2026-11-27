//! 公网网页的一页：钉页和栈里的入口。
use url::Url;
use wonton_protocol::{
    CoreError, PlacedTab, PluginsViewModel, TabIcon, TabLayout, TabPage, TabRef, TabState,
};

use super::{
    PagesState, StoredItem, StoredPage, invalid, is_https_url, load_failed_details,
    page_from_metadata,
};
use crate::l10n::{CATALOG, language};
use wonton_l10n::l10n;

/// 公网网页的一项：网址照原样，非 https 的不打开并说清原因。房间账号数据由别的客户端直接写，
/// 不经过入口校验，所以加载时也查一遍（ADR-0052 只收 `https://`）。
pub(crate) fn build_url_page(
    state: &PagesState,
    placed: &PlacedTab,
    stored: &StoredItem,
    url: &str,
    plugins: &PluginsViewModel,
) -> TabPage {
    let host = || {
        url.parse::<Url>()
            .ok()
            .and_then(|parsed| parsed.host_str().map(str::to_owned))
            .unwrap_or_else(|| url.to_owned())
    };
    let metadata = stored.page.clone().unwrap_or_else(|| StoredPage {
        plugin_title: host(),
        title: host(),
        icon: TabIcon::Plugin { icon: None },
        layout: TabLayout::Inset,
    });
    // 公网网页不经隧道，连不上是手机自己的网络，按上游出错显示（ADR-0052）。
    let https = is_https_url(url);
    let state_value = if https {
        match state.failures.get(placed) {
            Some((failure, status)) => TabState::LoadFailed {
                failure: *failure,
                status: *status,
                details: load_failed_details(*failure, *status),
            },
            None => TabState::Ready,
        }
    } else {
        TabState::Failed {
            reason: l10n!(CATALOG, language(), "panel-pin-not-https"),
        }
    };
    page_from_metadata(
        state,
        placed,
        metadata,
        https.then(|| url.to_owned()),
        state_value,
        plugins,
    )
}

pub(crate) fn stored_url_item(
    tab: &TabRef,
    url: &str,
    title: Option<String>,
) -> Result<StoredItem, CoreError> {
    if !is_https_url(url) {
        return Err(invalid(&l10n!(CATALOG, language(), "panel-pin-not-https")));
    }
    let host = url
        .parse::<Url>()
        .ok()
        .and_then(|parsed| parsed.host_str().map(str::to_owned));
    Ok(StoredItem {
        tab: tab.clone(),
        page: Some(StoredPage {
            plugin_title: host.clone().unwrap_or_else(|| url.to_owned()),
            title: title.or(host).unwrap_or_else(|| url.to_owned()),
            icon: TabIcon::Plugin { icon: None },
            layout: TabLayout::Inset,
        }),
    })
}
