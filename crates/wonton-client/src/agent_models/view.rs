//! 从插件给的目录算出页面上的行和点开那家的按钮。

use wonton_protocol::{
    ModelAuth, ModelCatalog, ModelKeyForm, ModelListState, ModelProvider, ModelProviderPage,
    ModelProviderRow,
};

/// 这家有没有 App 认识的接法；一种都不认识就不列（插件比 App 新）。
fn usable(provider: &ModelProvider) -> bool {
    provider
        .auth
        .iter()
        .any(|auth| !matches!(auth, ModelAuth::Other))
}

fn row(provider: &ModelProvider) -> ModelProviderRow {
    ModelProviderRow {
        id: provider.id.clone(),
        name: provider.name.clone(),
        connected: provider.connected,
        account: provider.account.clone(),
    }
}

fn matches(provider: &ModelProvider, search: &str) -> bool {
    let search = search.trim().to_lowercase();
    search.is_empty()
        || provider.name.to_lowercase().contains(&search)
        || provider.id.to_lowercase().contains(&search)
}

/// 推荐的照插件的顺序全列；其余的按搜索词过滤。
pub(super) fn rows(
    catalog: &ModelCatalog,
    search: &str,
) -> (Vec<ModelProviderRow>, Vec<ModelProviderRow>) {
    let listed = catalog.providers.iter().filter(|provider| usable(provider));
    let (recommended, more): (Vec<_>, Vec<_>) = listed.partition(|provider| provider.recommended);
    (
        recommended.into_iter().map(row).collect(),
        more.into_iter()
            .filter(|provider| matches(provider, search))
            .map(row)
            .collect(),
    )
}

pub(super) fn find<'a>(catalog: &'a ModelCatalog, id: &str) -> Option<&'a ModelProvider> {
    catalog
        .providers
        .iter()
        .find(|provider| provider.id == id && usable(provider))
}

/// 新点开一家：接好了的先拨成“正在取模型”，由调用方去取。
pub(super) fn page(provider: &ModelProvider) -> ModelProviderPage {
    let mut page = ModelProviderPage {
        row: row(provider),
        key: None,
        device_login: false,
        browser_login: false,
        login: None,
        login_outcome: None,
        busy: false,
        key_check: None,
        models: ModelListState::NotConnected,
        confirm: None,
    };
    refresh(&mut page, provider);
    page
}

/// 目录重新取回来后换掉这家的行和接法；断开了就清掉模型列表。返回要不要去取模型（刚接好、还没取过）。
pub(super) fn refresh(page: &mut ModelProviderPage, provider: &ModelProvider) -> bool {
    page.row = row(provider);
    page.key = None;
    page.device_login = false;
    page.browser_login = false;
    for auth in &provider.auth {
        match auth {
            ModelAuth::ApiKey {
                key_url,
                key_prefix,
                base_url,
            } => {
                page.key.get_or_insert(ModelKeyForm {
                    key_url: key_url.clone(),
                    key_prefix: key_prefix.clone(),
                    base_url: base_url.clone(),
                });
            }
            ModelAuth::DeviceLogin => page.device_login = true,
            ModelAuth::BrowserLogin => page.browser_login = true,
            ModelAuth::Other => {}
        }
    }
    if !provider.connected {
        page.models = ModelListState::NotConnected;
        return false;
    }
    if page.models == ModelListState::NotConnected {
        page.models = ModelListState::Loading;
        return true;
    }
    false
}
