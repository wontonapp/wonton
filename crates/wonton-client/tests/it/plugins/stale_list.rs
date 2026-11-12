//! 晚到的旧已安装列表不能把卸载掉的插件写回（盲测 #12：卸载后首页又出现、横幅 1→2、侧栏没有）。

use std::time::Duration;

use axum::http::Method;

use wonton_client::{Action, InstalledPlugins, PluginTaskState, StoreListing, StoreSection};

use super::rig::{Canned, Rig, item, running};

fn ids(view: &wonton_client::PluginsViewModel) -> Vec<String> {
    view.installed
        .as_ref()
        .map(|installed| installed.plugins.iter().map(|p| p.id.clone()).collect())
        .unwrap_or_default()
}

fn updatable_ids(view: &wonton_client::PluginsViewModel) -> Vec<String> {
    view.updatable
        .iter()
        .map(|item| item.plugin_id.clone())
        .collect()
}

/// 商店：magpie 有 2.0.0 可升（横幅里的“1 个可更新”），Miniflux 装着的就是商店这份最新版。
fn store_listing() -> StoreListing {
    let mut magpie = item("app.usemagpie.magpie", "Magpie", &["模型"]);
    magpie.version = "2.0.0".to_owned();
    magpie.installed_version = Some("1.0.0".to_owned());
    let mut miniflux = item("app.miniflux.miniflux", "Miniflux", &["阅读"]);
    miniflux.installed_version = Some("1.0.0".to_owned());
    let section = StoreSection {
        plugins: vec![magpie, miniflux],
        fetched_at_ms: None,
        problem: None,
        unreadable: Vec::new(),
    };
    StoreListing {
        official: section,
        added: Vec::new(),
        community: StoreSection {
            plugins: Vec::new(),
            fetched_at_ms: None,
            problem: None,
            unreadable: Vec::new(),
        },
    }
}

#[tokio::test]
async fn a_late_stale_list_does_not_bring_the_uninstalled_plugin_back() {
    let rig = Rig::new().await.without_periodic_checks();
    let magpie = running("app.usemagpie.magpie");
    let mut miniflux = running("app.miniflux.miniflux");
    miniflux.web_url = Some("http://app-miniflux.wonton.localhost:18480/".to_owned());
    let l0 = vec![magpie, miniflux];
    let l1 = vec![running("app.usemagpie.magpie")];
    rig.core.set_lists(vec![l0.clone(), l1.clone()]);
    rig.core.answer_always(
        Method::GET,
        "/_wonton/api/store",
        Canned(200, serde_json::to_string(&store_listing()).unwrap()),
    );
    rig.connected().await;
    // 连上只会自动取已安装列表；商店要主人进过才取。
    rig.app
        .dispatch(Action::RefreshStore { force: false })
        .unwrap();

    let view = rig
        .wait_for(|view| view.installed.is_some() && view.store.is_some())
        .await;
    assert_eq!(
        ids(&view),
        vec!["app.usemagpie.magpie", "app.miniflux.miniflux"]
    );
    assert_eq!(updatable_ids(&view), vec!["app.usemagpie.magpie"]);

    // 旧列表（还装着 Miniflux）发得早、回得晚：这个请求马上取走回应、睡 2 秒才回，卸载后的新列表先到。
    rig.core.answer_delayed(
        Method::GET,
        "/_wonton/api/plugins",
        Canned(
            200,
            serde_json::to_string(&InstalledPlugins {
                plugins: l0,
                upgrade_check: None,
            })
            .unwrap(),
        ),
        Duration::from_secs(2),
    );
    rig.app.dispatch(Action::RefreshPlugins).unwrap();
    tokio::time::sleep(Duration::from_millis(500)).await;

    rig.app
        .dispatch(Action::UninstallPlugin {
            id: "app.miniflux.miniflux".to_owned(),
            delete_data: false,
        })
        .unwrap();
    let state = rig
        .task("app.miniflux.miniflux", |state| {
            !matches!(state, PluginTaskState::Running)
        })
        .await;
    assert!(matches!(state, PluginTaskState::Succeeded), "{state:?}");
    let view = rig
        .wait_for(|view| ids(view) == vec!["app.usemagpie.magpie"])
        .await;
    assert_eq!(updatable_ids(&view), vec!["app.usemagpie.magpie"]);

    // 慢的旧列表到了：不能把 Miniflux 写回已安装列表，派生字段（横幅数、侧栏）也不能跟着回退。
    tokio::time::sleep(Duration::from_secs(5)).await;
    let view = rig.app.plugins();
    assert_eq!(
        ids(&view),
        vec!["app.usemagpie.magpie"],
        "晚到的旧列表把卸载掉的插件写回了已安装列表"
    );
    assert_eq!(updatable_ids(&view), vec!["app.usemagpie.magpie"]);
}
