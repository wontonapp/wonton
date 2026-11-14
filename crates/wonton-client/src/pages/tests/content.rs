use super::*;
use crate::pages::bridge::{self, TabEnvironment, TabPlatform, TabSafeArea};
use crate::pages::media_permission::TabMediaPermission;

fn place() -> PlacedTab {
    PlacedTab::Content { tab: page_tab() }
}

#[tokio::test]
async fn independent_content_preserves_stack_and_shares_loading_state() {
    let rig = Rig::new();
    let first = rig.tabs.subscribe_content_page(page_tab()).unwrap();
    assert!(rig.view().stack.is_empty());
    assert_eq!(first.page().state, TabState::Ready);
    assert_eq!(first.next().await.unwrap(), first.page());
    rig.open(page_tab());
    let second = rig.tabs.subscribe_content_page(page_tab()).unwrap();
    rig.tabs.load_failed(place(), Some(403)).unwrap();
    assert_eq!(first.next().await.unwrap(), rig.page(&page_tab()));
    assert_eq!(second.page(), first.page());
    assert!(matches!(
        first.page().state,
        TabState::LoadFailed {
            status: Some(403),
            ..
        }
    ));
    let reload = first.page().reload;
    rig.tabs.reload(place()).unwrap();
    assert_eq!(first.page().state, TabState::Ready);
    assert_eq!(first.page().reload, reload + 1);
    assert_eq!(rig.page(&page_tab()), first.page());
    first.cancel();
    first.cancel();
    assert!(first.next().await.is_none());
    rig.tabs.close_page().unwrap();
    assert!(rig.view().stack.is_empty());
    rig.tabs.load_failed(place(), None).unwrap();
    assert!(matches!(
        second.page().state,
        TabState::LoadFailed {
            failure: TabLoadFailure::Transport,
            ..
        }
    ));
    drop(second);
    assert!(bridge::page_of(&rig.tabs, &place()).is_err());
    assert!(rig.tabs.load_succeeded(place()).is_err());
    assert!(rig.tabs.reload(place()).is_err());
}

#[tokio::test]
async fn content_tracks_plugin_restart_removal_and_connection() {
    let rig = Rig::new();
    let content = rig.tabs.subscribe_content_page(page_tab()).unwrap();
    rig.plugins.send_modify(|view| {
        view.installed.as_mut().unwrap().plugins[0].status = PluginState::Stopped;
    });
    rig.tabs.recompute();
    assert_eq!(content.page().state, TabState::Stopped);
    let reload = content.page().reload;
    rig.plugins.send_replace(plugins("new-content"));
    rig.tabs.recompute();
    assert_eq!(content.page().state, TabState::Ready);
    assert_eq!(content.page().reload, reload + 1);
    rig.tabs.load_failed(place(), None).unwrap();
    rig.home.send_modify(|home| {
        home.status = HomeStatus::Disconnected {
            reason: HomeFailure::TransportUnavailable,
            message: "offline".to_owned(),
        }
    });
    rig.tabs.recompute();
    rig.home
        .send_modify(|home| home.status = HomeStatus::Connected);
    rig.tabs.recompute();
    assert_eq!(content.page().state, TabState::Ready);
    assert_eq!(content.page().reload, reload + 2);
    rig.plugins
        .send_modify(|view| view.installed.as_mut().unwrap().plugins.clear());
    rig.tabs.recompute();
    assert_eq!(content.page().state, TabState::PageRemoved);
    assert!(content.page().url.is_none());
    assert!(rig.view().stack.is_empty());
}

fn environment() -> TabEnvironment {
    TabEnvironment {
        locale: "en".to_owned(),
        time_zone: "UTC".to_owned(),
        platform: TabPlatform::Ios,
        dark: false,
        inset_top: 0.0,
        inset_bottom: 0.0,
        safe_area: TabSafeArea {
            top: 0.0,
            right: 0.0,
            bottom: 0.0,
            left: 0.0,
        },
    }
}

#[tokio::test]
async fn content_bridge_and_media_require_live_lease_and_exact_origin() {
    let rig = Rig::new();
    let appearance = crate::appearance::AppearanceLink::new(rig._dir.path());
    let first = rig.tabs.subscribe_content_page(page_tab()).unwrap();
    let second = rig.tabs.subscribe_content_page(page_tab()).unwrap();
    let origin = "http://app.shared.wonton.localhost";
    let message = r#"{"jsonrpc":"2.0","method":"ui/notifications/initialized"}"#;
    let initialized = |origin: &str, main: bool| {
        bridge::handle_message(
            &rig.tabs,
            &appearance,
            &place(),
            main,
            origin,
            message,
            &environment(),
        )
        .initialized
    };
    assert!(initialized(origin, true));
    assert!(!initialized("https://app.shared.wonton.localhost", true));
    assert!(!initialized("http://app.shared.wonton.localhost:81", true));
    assert!(!initialized(origin, false));
    assert_eq!(
        rig.tabs
            .request_media_permission(&place(), "https://evil.example", true, false)
            .unwrap(),
        TabMediaPermission::Deny
    );
    assert_eq!(
        rig.tabs
            .request_media_permission(&place(), origin, true, false)
            .unwrap(),
        TabMediaPermission::Prompt
    );
    first.cancel();
    assert!(initialized(origin, true));
    assert!(rig.view().media_prompt.is_some());
    second.cancel();
    assert!(!initialized(origin, true));
    assert!(rig.view().media_prompt.is_none());
    assert_eq!(
        rig.tabs
            .request_media_permission(&place(), origin, true, false)
            .unwrap(),
        TabMediaPermission::Deny
    );
    assert!(
        bridge::document_script(&rig.tabs, &appearance.view(), &place(), &environment()).is_err()
    );
}

#[tokio::test]
async fn public_content_retains_https_and_no_bridge_policy() {
    let rig = Rig::new();
    let tab = TabRef::Url {
        url: "https://example.org/path".to_owned(),
    };
    let content = rig.tabs.subscribe_content_page(tab.clone()).unwrap();
    let placed = PlacedTab::Content { tab };
    let appearance = crate::appearance::AppearanceLink::new(rig._dir.path());
    assert_eq!(
        content.page().url.as_deref(),
        Some("https://example.org/path")
    );
    assert!(
        bridge::document_script(&rig.tabs, &appearance.view(), &placed, &environment()).is_err()
    );
    assert_eq!(
        rig.tabs
            .request_media_permission(&placed, "https://example.org", true, false)
            .unwrap(),
        TabMediaPermission::Deny
    );
    assert!(
        rig.tabs
            .subscribe_content_page(TabRef::Url {
                url: "http://example.org".to_owned()
            })
            .is_err()
    );
    assert!(
        rig.tabs
            .subscribe_content_page(TabRef::PluginPath {
                plugin_id: "app.shared".to_owned(),
                path: "https://evil.example".to_owned()
            })
            .is_err()
    );
    assert!(rig.view().stack.is_empty());
}

#[tokio::test]
async fn content_uses_the_same_browser_policy_as_stack_pages() {
    let rig = Rig::new();
    rig.plugins.send_modify(|view| {
        view.installed.as_mut().unwrap().plugins[0].cross_origin_isolated = true;
    });
    rig.tabs.recompute();
    let content = rig.tabs.subscribe_content_page(page_tab()).unwrap();
    rig.open(page_tab());
    assert_eq!(content.page(), rig.page(&page_tab()));
    assert_eq!(content.page().opens_in_browser, cfg!(target_os = "android"));
    drop(content);
    assert!(bridge::page_of(&rig.tabs, &place()).is_err());
    assert_eq!(rig.page(&page_tab()).state, TabState::Ready);
    rig.tabs
        .reload(PlacedTab::Page { tab: page_tab() })
        .unwrap();
}

#[tokio::test]
async fn cancellation_wakes_a_waiting_content_receiver() {
    let rig = Rig::new();
    let content = rig.tabs.subscribe_content_page(page_tab()).unwrap();
    content.next().await.unwrap();
    let waiting = content.next();
    tokio::pin!(waiting);
    assert!(futures_util::poll!(&mut waiting).is_pending());
    content.cancel();
    assert!(waiting.await.is_none());
}
