//! 假数据入口里写死的商店列表：官方索引里三项，行上可以点进详情页。
//! 没有后端，安装、刷新照旧落不到作用域；界面测试看商店列表和商店详情怎么画、页面之间怎么来回。

use wonton_protocol::{PluginDetail, PluginDetailRoute, StoreItem, StoreListing, StoreSection};

pub(super) const VAULT_TITLE: &str = "Photo Vault";
pub(super) const VAULT_SOURCE: &str = "github:example/wonton-vault";
pub(super) const MUSIC_TITLE: &str = "Home Music";
pub(super) const MUSIC_SOURCE: &str = "github:example/wonton-music";
const LAN_SOURCE: &str = "github:example/wonton-lan-scan";

/// 商店列表：第一项已经装着，后两项没装。
pub(super) fn listing() -> StoreListing {
    StoreListing {
        official: StoreSection {
            plugins: vec![
                item(VAULT_TITLE, VAULT_SOURCE, Some("0.9.0")),
                item(MUSIC_TITLE, MUSIC_SOURCE, None),
            ],
            fetched_at_ms: Some(1_793_000_000_000),
            problem: None,
            unreadable: Vec::new(),
        },
        added: Vec::new(),
        community: StoreSection {
            plugins: {
                let mut bangumi =
                    item("Autobangumi", super::ui_fake_install::CONSUMER_SOURCE, None);
                bangumi.id = super::ui_fake_install::store_id();
                let mut downloader =
                    item("qBittorrent", super::ui_fake_install::PROVIDER_SOURCE, None);
                downloader.id = super::ui_fake_install::provider_store_id();
                vec![item("LAN Scan", LAN_SOURCE, None), bangumi, downloader]
            },
            fetched_at_ms: Some(1_793_000_000_000),
            problem: None,
            unreadable: Vec::new(),
        },
    }
}

fn item(title: &str, source: &str, installed_version: Option<&str>) -> StoreItem {
    StoreItem {
        source: source.into(),
        id: id_of(source),
        title: title.into(),
        summary: "The household's own copies, on the home server".into(),
        icon: None,
        tags: Vec::new(),
        kind: wonton_protocol::PluginKind::App,
        version: "1.0.0".into(),
        trust: wonton_protocol::PluginTrust::Official,
        provider: Some(format!("https://{}", source.trim_start_matches("github:"))),
        via_index: None,
        author: None,
        license: Some("MIT".into()),
        license_url: None,
        stars: None,
        installed_version: installed_version.map(Into::into),
        security_tier: None,
        blocked: None,
        installable: true,
        install_gate: wonton_protocol::Gate::Allow,
        high_permissions: false,
    }
}

/// 商店详情的测试数据：两项都有自己的详情，别的来源照旧没有。
pub(super) fn detail(route: &PluginDetailRoute) -> Option<PluginDetail> {
    let PluginDetailRoute::Store { source } = route else {
        return None;
    };
    let (title, summary) = match source.as_str() {
        VAULT_SOURCE => (
            VAULT_TITLE,
            "Every photo the household takes, on the home server",
        ),
        MUSIC_SOURCE => (
            MUSIC_TITLE,
            "The household's records, playable from anywhere",
        ),
        super::ui_fake_install::CONSUMER_SOURCE => {
            ("Autobangumi", "Follows the season and grabs new episodes")
        }
        _ => return None,
    };
    let mut detail = super::ui_fake_plugin_detail::app(&id_of(source), title, summary, None);
    detail.source = Some(source.clone());
    // 商店那一行和装着的那一份是不同的 ID：不然详情会被当成已装的，只给“打开”。
    if source == super::ui_fake_install::CONSUMER_SOURCE {
        detail.id = super::ui_fake_install::store_id();
    } else if source == super::ui_fake_install::PROVIDER_SOURCE {
        detail.id = super::ui_fake_install::provider_store_id();
    }
    Some(detail)
}

fn id_of(source: &str) -> String {
    format!(
        "app.example.{}",
        source.rsplit('/').next().unwrap_or_default()
    )
}
