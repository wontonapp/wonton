//! 主人主动看有没有新版本时（升级预览、手动刷新商店），官方索引不等 6 小时的缓存过期，没取到如实报；
//! 按 tag 装的社区插件，官方索引收录了同一个提交后认成官方。

use std::fs;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use wonton_protocol::{InstallRequest, PluginTrust};

use super::super::docker::Engine;
use super::super::fake_docker::FakeDocker;
use super::super::github::GitHub;
use super::super::storefront::index::IndexEntry;
use super::super::storefront::{MIN_RETRY, StoreConfig};
use super::super::{PluginId, PluginManager, PluginPackage, UpgradeTo};
use super::github::{FakeGitHub, ICON, compose, release, zh};
use super::manager::manager_with;

/// 用 GitHub 上的官方索引的核心。
struct Online {
    github: FakeGitHub,
    client: GitHub,
    engine: Engine,
    m: Arc<PluginManager>,
    data: tempfile::TempDir,
}

fn manager(engine: Engine, client: GitHub, data: &tempfile::TempDir) -> PluginManager {
    let config = StoreConfig {
        index_dir: None,
        community: false,
    };
    manager_with(engine, data)
        .with_github(client)
        .with_store(config)
}

async fn online() -> Online {
    let (github, client) = FakeGitHub::start().await;
    let (_docker, engine) = FakeDocker::start().await;
    let data = tempfile::tempdir().unwrap();
    let m = Arc::new(manager(engine.clone(), client.clone(), &data));
    Online {
        github,
        client,
        engine,
        m,
        data,
    }
}

/// 官方索引只收录 `acme/hello` 在 `commit` 上的 `version` 版。
fn publish_index(github: &FakeGitHub, version: &str, commit: &str) {
    let files = [("icon.svg".to_owned(), ICON.as_bytes().to_vec())].into();
    let package =
        PluginPackage::with_files(&compose(version, "nginx"), files, &super::langs()).unwrap();
    let entry = IndexEntry::of(&package, "github:acme/hello", commit).unwrap();
    let dir = tempfile::tempdir().unwrap();
    entry.write_to(dir.path(), &package).unwrap();
    github.publish_index(fs::read(dir.path().join("index.json")).unwrap());
}

/// 过了一分钟：又可以去取官方索引了，6 小时的缓存还没过期。
async fn a_minute_later(m: &PluginManager) {
    m.storefront.age_official(MIN_RETRY).await;
}

async fn install(m: &Arc<PluginManager>, source: &str) {
    let preview = m.preview_source(source, &zh()).await.unwrap();
    let request = InstallRequest {
        scope: None,
        audience: None,
        source: preview.source.unwrap(),
        digest: preview.digest,
        plan: preview.plan,
        grant: Vec::new(),
        enabled_permissions: Vec::new(),
        secrets: Default::default(),
        settings: Default::default(),
        devices: Vec::new(),
        lan_webui: false,
        install_together: Vec::new(),
        connect_plugins: Vec::new(),
    };
    m.install_source(&request).await.unwrap();
}

fn hello() -> PluginId {
    PluginId::parse("app.hello").unwrap()
}

fn trust(m: &PluginManager) -> PluginTrust {
    m.installed_plugin(&hello(), &zh()).unwrap().trust
}

#[tokio::test]
async fn the_upgrade_preview_fetches_the_index_again_and_says_when_it_cannot() {
    let o = online().await;
    let first = release(&o.github, "1.0.0", "nginx");
    publish_index(&o.github, "1.0.0", &first);
    install(&o.m, "github:acme/hello").await;
    assert_eq!(trust(&o.m), PluginTrust::Official);
    let second = release(&o.github, "1.1.0", "nginx");
    publish_index(&o.github, "1.1.0", &second);

    // 离上次去取不到一分钟：不重复取。
    let err =
        o.m.preview_upgrade(&hello(), UpgradeTo::Offered, &zh())
            .await
            .unwrap_err();
    assert_eq!(err.kind(), "not_found", "{err}");

    // 取不到：上次的索引里没有新版本，不能说成没有新版本。一分钟内再试也一样。
    a_minute_later(&o.m).await;
    o.github.broken.store(true, Ordering::Relaxed);
    for _ in 0..2 {
        let err =
            o.m.preview_upgrade(&hello(), UpgradeTo::Offered, &zh())
                .await
                .unwrap_err();
        assert_eq!(err.kind(), "source_unavailable", "{err}");
        o.github.broken.store(false, Ordering::Relaxed);
    }

    a_minute_later(&o.m).await;
    let preview =
        o.m.preview_upgrade(&hello(), UpgradeTo::Offered, &zh())
            .await
            .unwrap();
    assert_eq!(preview.version, "1.1.0");
    assert_eq!(preview.trust, PluginTrust::Official);
}

#[tokio::test]
async fn a_tagged_install_becomes_official_once_the_index_lists_its_commit() {
    let o = online().await;
    let commit = release(&o.github, "1.0.0", "nginx");
    // 官方索引还没收录：按 tag 从 GitHub 装，是社区的。
    install(&o.m, "github:acme/hello@v1.0.0").await;
    assert_eq!(trust(&o.m), PluginTrust::Community);

    // 收录的是同一个版本的另一个提交：装着的不是审核过的那份，保持社区。
    publish_index(&o.github, "1.0.0", &"d".repeat(40));
    a_minute_later(&o.m).await;
    o.m.spawn_upgrade_check(true).await.unwrap();
    assert_eq!(trust(&o.m), PluginTrust::Community);

    // 主人手动刷新商店：索引收录了装着的提交，认成官方，不用重装。
    publish_index(&o.github, "1.0.0", &commit);
    a_minute_later(&o.m).await;
    o.m.store_listing(&zh(), true).await;
    o.m.check_upgrades_now().await;
    assert_eq!(trust(&o.m), PluginTrust::Official);
    let installed = o.m.installed_plugin(&hello(), &zh()).unwrap();
    assert_eq!(
        installed.source.as_deref(),
        Some("github:acme/hello@v1.0.0")
    );

    // 记下来了：核心重启后照样是官方的。
    let restarted = manager(o.engine, o.client, &o.data);
    restarted.restore().await.unwrap();
    assert_eq!(trust(&restarted), PluginTrust::Official);
}
