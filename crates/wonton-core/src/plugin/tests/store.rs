//! 商店（插件规范的“官方索引”“社区插件”）：本地的索引目录、按索引核对的官方插件、GitHub 上的社区插件。

use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use super::super::fake_docker::FakeDocker;
use super::super::gallery::Variant;
use super::super::package_files::sha256_hex;
use super::super::storefront::StoreConfig;
use super::super::storefront::index::IndexEntry;
use super::super::{PluginId, PluginManager, PluginPackage};
use super::github::{FakeGitHub, ICON, compose, release, zh};
use super::manager::manager_with;
use serde_json::{Value, json};
use wonton_protocol::{InstallRequest, PluginTrust, StoreFailure};

/// 在 `dir` 里放一份官方索引：`acme/hello` 的 1.0.0 版，文件按 sha256 放在 `blobs/` 下（同 `plugin index-entry`）。
fn write_index(dir: &Path, compose: &str, icon: &str) {
    let files = [("icon.svg".to_owned(), icon.as_bytes().to_vec())].into();
    let package = PluginPackage::with_files(compose, files).unwrap();
    let entry = IndexEntry::of(&package, "github:acme/hello", &"c".repeat(40)).unwrap();
    entry.write_to(dir, &package).unwrap();
}

struct Bed {
    m: Arc<PluginManager>,
    github: FakeGitHub,
    _dir: tempfile::TempDir,
}

async fn bed(index: &Path, community: bool) -> Bed {
    let (github, client) = FakeGitHub::start().await;
    let (_docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let config = StoreConfig {
        index_dir: Some(index.to_owned()),
        community,
    };
    let m = manager_with(engine, &dir)
        .with_github(client)
        .with_store(config);
    Bed {
        m: Arc::new(m),
        github,
        _dir: dir,
    }
}

#[tokio::test]
async fn official_plugins_come_from_the_index_and_are_checked_against_it() {
    let index = tempfile::tempdir().unwrap();
    write_index(index.path(), &compose("1.0.0", "nginx"), ICON);
    // 同一个来源再写一次：换掉旧的一项，不重复。
    write_index(index.path(), &compose("1.0.0", "nginx"), ICON);
    let bed = bed(index.path(), false).await;
    let m = &bed.m;

    let listing = m.store_listing(&zh(), false).await;
    let [item] = &listing.official.plugins[..] else {
        panic!("{listing:?}");
    };
    assert_eq!(
        (item.source.as_str(), item.trust),
        ("github:acme/hello", PluginTrust::Official)
    );
    assert_eq!(listing.official.problem, None);
    // 列出的图标能取到。
    let sha256 = item
        .icon
        .as_ref()
        .unwrap()
        .small
        .path
        .split('/')
        .nth(4)
        .unwrap()
        .to_owned();
    assert!(m.icon_png(&sha256, Variant::Small).await.unwrap().is_some());

    let (_, _, onboarding) = m.onboarding_store_listing(&zh()).await;
    assert!(onboarding[0].icon.is_some());

    // 不带版本、或者就是索引里的版本：官方的，没有“未审核”的提示，不用去 GitHub 取。
    let preview = m
        .preview_source("github:acme/hello@v1.0.0", &zh())
        .await
        .unwrap();
    assert_eq!(preview.trust, PluginTrust::Official);
    assert!(
        preview.notices.iter().all(|n| !n.emphasis),
        "{:?}",
        preview.notices
    );
    let request = InstallRequest {
        source: preview.source.unwrap(),
        digest: preview.digest,
        grant: Vec::new(),
        devices: Vec::new(),
        lan_webui: false,
    };
    m.install_source(&request).await.unwrap();
    let id = PluginId::parse("app.hello").unwrap();
    assert_eq!(
        m.installed_plugin(&id, &zh()).unwrap().trust,
        PluginTrust::Official
    );
    let listing = m.store_listing(&zh(), false).await;
    assert_eq!(
        listing.official.plugins[0].installed_version.as_deref(),
        Some("1.0.0")
    );
}

#[tokio::test]
async fn files_that_do_not_match_the_index_are_refused() {
    let index = tempfile::tempdir().unwrap();
    write_index(index.path(), &compose("1.0.0", "nginx"), ICON);
    // 图标的文件被换了。
    fs::write(
        index.path().join("blobs").join(sha256_hex(ICON.as_bytes())),
        "<svg/>",
    )
    .unwrap();
    let bed = bed(index.path(), false).await;
    let m = &bed.m;
    let err = m
        .preview_source("github:acme/hello", &zh())
        .await
        .unwrap_err();
    assert_eq!(err.kind(), "invalid_input", "{err}");

    // compose 和 files 里记的对不上：这一项不列出，原因告诉主人。
    let other = compose("1.0.0", "evil");
    let mut raw: serde_json::Value =
        serde_json::from_slice(&fs::read(index.path().join("index.json")).unwrap()).unwrap();
    raw["plugins"][0]["compose"] = json!(other);
    fs::write(index.path().join("index.json"), raw.to_string()).unwrap();
    let listing = m.store_listing(&zh(), false).await;
    assert!(listing.official.plugins.is_empty());
    assert_eq!(listing.official.unreadable[0].source, "github:acme/hello");

    // 清单有核心不认的字段：同样告诉主人是哪一个、为什么。
    let unknown =
        compose("1.0.0", "nginx").replace("x-wonton:\n", "x-wonton:\n  category: music\n");
    let mut raw: serde_json::Value =
        serde_json::from_slice(&fs::read(index.path().join("index.json")).unwrap()).unwrap();
    raw["plugins"][0]["compose"] = json!(unknown);
    raw["plugins"][0]["files"]["docker-compose.yml"] = json!(sha256_hex(unknown.as_bytes()));
    fs::write(index.path().join("index.json"), raw.to_string()).unwrap();
    let listing = m.store_listing(&zh(), false).await;
    assert!(listing.official.plugins.is_empty());
    assert_eq!(listing.official.problem, None);
    assert!(listing.official.unreadable[0].reason.contains("category"));

    // 格式比核心新：读不懂，照实说明。
    fs::write(
        index.path().join("index.json"),
        r#"{"format": 2, "plugins": []}"#,
    )
    .unwrap();
    let listing = m.store_listing(&zh(), false).await;
    assert_eq!(
        listing.official.problem.unwrap().reason,
        StoreFailure::IndexTooNew
    );

    fs::write(index.path().join("index.json"), "不是 JSON").unwrap();
    let listing = m.store_listing(&zh(), false).await;
    assert_eq!(
        listing.official.problem.unwrap().reason,
        StoreFailure::InvalidIndex
    );
}

#[tokio::test]
async fn community_plugins_are_listed_by_stars_without_official_ones() {
    let index = tempfile::tempdir().unwrap();
    write_index(index.path(), &compose("1.0.0", "nginx"), ICON);
    let bed = bed(index.path(), true).await;
    let (m, github) = (&bed.m, &bed.github);
    // acme/hello 是官方的；bob/hello 的插件 ID 和它一样；carol/empty 没有正式版本。
    release(github, "1.1.0", "nginx");
    let tool = compose("2.0.0", "nginx").replace("app.hello", "app.tool");
    github.publish(
        "bob/tool",
        "v2.0.0",
        &[("docker-compose.yml", &tool), ("icon.svg", ICON)],
    );
    let same_id = compose("1.0.0", "nginx");
    github.publish("bob/hello", "v1.0.0", &[("docker-compose.yml", &same_id)]);
    github.publish("carol/empty", "nightly", &[]);
    let old = compose("1.0.0", "nginx")
        .replace("app.hello", "app.old")
        .replace("x-wonton:\n", "x-wonton:\n  category: music\n");
    github.publish("dan/old", "v1.0.0", &[("docker-compose.yml", &old)]);
    *github.topic.lock().unwrap() = vec![
        ("acme/hello".into(), 50),
        ("bob/tool".into(), 9),
        ("bob/hello".into(), 3),
        ("carol/empty".into(), 1),
        ("dan/old".into(), 1),
    ];

    let listing = m.store_listing(&zh(), true).await;
    let community: Vec<_> = listing
        .community
        .plugins
        .iter()
        .map(|p| (p.source.as_str(), p.trust, p.stars))
        .collect();
    assert_eq!(
        community,
        [("github:bob/tool", PluginTrust::Community, Some(9))]
    );
    assert!(listing.community.fetched_at_ms.is_some());
    assert_eq!(listing.community.problem, None);
    let unreadable = &listing.community.unreadable;
    assert_eq!(unreadable.len(), 1, "没有正式版本的 carol/empty 不算读不懂");
    assert_eq!(unreadable[0].source, "github:dan/old");
    assert!(unreadable[0].reason.contains("category"));
    assert!(
        !github
            .tag_reads
            .lock()
            .unwrap()
            .contains(&"acme/hello".to_owned()),
        "官方插件不为了列表去读仓库"
    );

    let detail = m.store_plugin("github:bob/tool", &zh()).await.unwrap();
    assert_eq!(
        (detail.version.as_str(), detail.trust),
        ("2.0.0", PluginTrust::Community)
    );
    assert_eq!(detail.stars, Some(9));
    let official = m.store_plugin("github:acme/hello", &zh()).await.unwrap();
    assert_eq!(
        (official.version.as_str(), official.trust),
        ("1.0.0", PluginTrust::Official)
    );
}

#[tokio::test]
async fn the_store_says_why_community_plugins_are_missing() {
    let index = tempfile::tempdir().unwrap();
    write_index(index.path(), &compose("1.0.0", "nginx"), ICON);
    let off = bed(index.path(), false).await;
    let problem = off
        .m
        .store_listing(&zh(), true)
        .await
        .community
        .problem
        .unwrap();
    assert_eq!(problem.reason, StoreFailure::Disabled);

    let (_docker, engine) = FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let offline = manager_with(engine, &dir)
        .with_github(super::super::github::GitHub::for_test("http://127.0.0.1:9"))
        .with_store(StoreConfig {
            index_dir: Some(index.path().to_owned()),
            community: true,
        });
    let listing = offline.store_listing(&zh(), true).await;
    assert_eq!(
        listing.community.problem.unwrap().reason,
        StoreFailure::Unreachable
    );
    // 官方插件在本地目录里，照常列出。
    assert_eq!(listing.official.plugins.len(), 1);
}

/// 用 GitHub 上的官方索引的核心：`data` 就是它的数据目录，换个管理器算重启。
async fn online(data: &tempfile::TempDir) -> (FakeGitHub, PluginManager) {
    let (github, client) = FakeGitHub::start().await;
    let (_docker, engine) = FakeDocker::start().await;
    let config = StoreConfig {
        index_dir: None,
        community: false,
    };
    let m = manager_with(engine, data)
        .with_github(client)
        .with_store(config);
    (github, m)
}

fn index_json(compose: &str) -> Vec<u8> {
    let dir = tempfile::tempdir().unwrap();
    write_index(dir.path(), compose, ICON);
    fs::read(dir.path().join("index.json")).unwrap()
}

#[tokio::test]
async fn the_last_index_survives_a_restart_and_says_why_it_is_old() {
    let data = tempfile::tempdir().unwrap();
    let (github, m) = online(&data).await;
    github.publish_index(index_json(&compose("1.0.0", "nginx")));
    let listing = m.store_listing(&zh(), true).await;
    assert_eq!(listing.official.plugins.len(), 1);
    assert_eq!(listing.official.problem, None);
    let fetched_at = listing.official.fetched_at_ms;
    assert!(fetched_at.is_some());
    drop(m);

    let (github, m) = online(&data).await;
    github.broken.store(true, Ordering::Relaxed);
    let listing = m.store_listing(&zh(), false).await;
    assert_eq!(listing.official.plugins.len(), 1, "用磁盘上的那份");
    assert_eq!(
        listing.official.fetched_at_ms, fetched_at,
        "时间是取到它的时候"
    );
    assert_eq!(
        listing.official.problem.unwrap().reason,
        StoreFailure::Unreachable
    );
}

#[tokio::test]
async fn a_broken_index_is_not_saved_and_a_hang_is_called_a_timeout() {
    let data = tempfile::tempdir().unwrap();
    let (github, m) = online(&data).await;
    github.publish_index(b"{}".to_vec());
    let listing = m.store_listing(&zh(), true).await;
    assert!(listing.official.plugins.is_empty());
    assert_eq!(listing.official.fetched_at_ms, None);
    assert_eq!(
        listing.official.problem.unwrap().reason,
        StoreFailure::InvalidIndex
    );
    assert!(
        !data
            .path()
            .join("plugins/store/official-index.json")
            .exists()
    );

    let (github, m) = online(&data).await;
    github.hanging.store(true, Ordering::Relaxed);
    let listing = m.store_listing(&zh(), true).await;
    assert_eq!(
        listing.official.problem.unwrap().reason,
        StoreFailure::Timeout
    );
}

#[tokio::test]
async fn the_index_comes_through_the_api_when_raw_is_down() {
    let data = tempfile::tempdir().unwrap();
    let (github, m) = online(&data).await;
    github.publish_index(index_json(&compose("1.0.0", "nginx")));
    github.raw_broken.store(true, Ordering::Relaxed);
    let listing = m.store_listing(&zh(), true).await;
    assert_eq!(listing.official.plugins.len(), 1);
    assert_eq!(listing.official.problem, None);

    let (github, m) = online(&tempfile::tempdir().unwrap()).await;
    github.raw_broken.store(true, Ordering::Relaxed);
    github.api_limited.store(true, Ordering::Relaxed);
    let listing = m.store_listing(&zh(), true).await;
    assert_eq!(
        listing.official.problem.unwrap().reason,
        StoreFailure::RateLimited
    );
}

#[tokio::test]
async fn onboarding_keeps_a_cached_catalog_when_the_official_source_breaks() {
    let data = tempfile::tempdir().unwrap();
    let (github, m) = online(&data).await;
    let mut index: Value = serde_json::from_slice(&index_json(&compose("1.0.0", "nginx"))).unwrap();
    index["onboarding"] = json!({
        "agents": [],
        "needs": [],
        "providers": {},
        "more_providers": [],
        "prompts": [{"en": "What can you help me with"}]
    });
    github.publish_index(serde_json::to_vec(&index).unwrap());
    let (first, first_problem, _) = m.onboarding_store_listing(&zh()).await;
    assert!(first.onboarding.is_some());
    assert_eq!(first_problem, None);
    drop(m);

    let (github, m) = online(&data).await;
    github.broken.store(true, Ordering::Relaxed);
    let (cached, problem, _) = m.onboarding_store_listing(&zh()).await;
    assert!(cached.onboarding.is_some());
    assert_eq!(problem.unwrap().reason, StoreFailure::Unreachable);
}

#[tokio::test]
async fn store_details_warn_about_isolation_before_install_in_the_requested_language() {
    use super::super::languages::Languages;

    for isolation in [None, Some(false), Some(true)] {
        let index = tempfile::tempdir().unwrap();
        let mut source = compose("1.0.0", "nginx");
        if let Some(value) = isolation {
            source.push_str(&format!(
                "  webui: {{service: web, port: 8080, cross_origin_isolated: {value}}}\n"
            ));
        }
        write_index(index.path(), &source, ICON);
        let bed = bed(index.path(), false).await;
        let mut localized = Vec::new();
        for language in ["zh-Hans", "en-US"] {
            let detail = bed
                .m
                .store_plugin("github:acme/hello", &Languages::from_header(Some(language)))
                .await
                .unwrap();
            assert!(detail.installed.is_none());
            if isolation == Some(true) {
                let [notice] = detail.notices.as_slice() else {
                    panic!("{detail:?}")
                };
                assert!(notice.emphasis);
                localized.push(notice.text.clone());
            } else {
                assert!(detail.notices.is_empty(), "{detail:?}");
            }
        }
        if isolation == Some(true) {
            assert_ne!(localized[0], localized[1]);
            assert!(!localized[0].is_ascii());
            assert!(localized[1].is_ascii());
        }
    }
}
