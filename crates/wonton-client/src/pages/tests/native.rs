//! 声明了浏览标准的插件：入口页换成原生界面，“网页版”照旧是网页。

use wonton_protocol::NativePage;
use wonton_protocol::media::{MediaBrowse, MediaOperation};

use super::*;

fn library(browse: Option<MediaBrowse>) -> PluginsViewModel {
    let mut library = plugin("app.shared", "Music", "1");
    library.media_browse = browse;
    installed(library)
}

fn media_source(operations: Vec<MediaOperation>) -> PluginsViewModel {
    let mut source = plugin("app.shared", "Movies", "1");
    source.media_operations = operations;
    installed(source)
}

fn installed(library: InstalledPlugin) -> PluginsViewModel {
    PluginsViewModel {
        installed: Some(InstalledPlugins {
            plugins: vec![library],
            upgrade_check: None,
        }),
        ..Default::default()
    }
}

#[tokio::test]
async fn a_browsable_library_opens_natively_and_its_web_page_stays_reachable() {
    let rig = Rig::new();
    rig.plugins
        .send_replace(library(Some(MediaBrowse::OpenSubsonic)));
    rig.open(page_tab());
    assert_eq!(rig.page(&page_tab()).native, Some(NativePage::Music));

    let web = TabRef::PluginPath {
        plugin_id: "app.shared".to_owned(),
        path: "/".to_owned(),
    };
    rig.open(web.clone());
    let page = rig.page(&web);
    assert_eq!(page.native, None);
    assert!(page.url.is_some());
}

#[tokio::test]
async fn a_standard_this_app_does_not_know_keeps_the_web_page() {
    let rig = Rig::new();
    rig.plugins
        .send_replace(library(Some(MediaBrowse::Other("x-acme".to_owned()))));
    rig.open(page_tab());
    let page = rig.page(&page_tab());
    assert_eq!(page.native, None);
    assert!(page.url.is_some());
}

#[tokio::test]
async fn a_catalog_provider_opens_the_video_page_and_its_web_page_stays_reachable() {
    let rig = Rig::new();
    rig.plugins.send_replace(media_source(vec![
        MediaOperation::Playback,
        MediaOperation::Catalog,
    ]));
    rig.open(page_tab());
    assert_eq!(rig.page(&page_tab()).native, Some(NativePage::Video));

    let web = TabRef::PluginPath {
        plugin_id: "app.shared".to_owned(),
        path: "/".to_owned(),
    };
    rig.open(web.clone());
    assert_eq!(rig.page(&web).native, None);
}

#[tokio::test]
async fn a_media_source_without_catalog_keeps_the_web_page() {
    let rig = Rig::new();
    rig.plugins.send_replace(media_source(vec![
        MediaOperation::Playback,
        MediaOperation::Other("x-acme".to_owned()),
    ]));
    rig.open(page_tab());
    assert_eq!(rig.page(&page_tab()).native, None);
}
