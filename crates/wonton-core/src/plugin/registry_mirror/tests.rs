use super::*;
use crate::plugin::languages::Languages;
use crate::plugin::store::Store;
use crate::plugin::storefront::index::Index;
use wonton_protocol::{RegistryFailure, RegistryProblem};

fn index(json: &str) -> Index {
    Index::parse(json.as_bytes(), &Languages::default()).unwrap()
}

#[test]
fn registry_index_reads_the_lists_per_registry() {
    let index = index(
        r#"{"format":1,"plugins":[],"registry_mirrors":{
            "docker.io":["docker.example:5000"],"ghcr.io":["ghcr.example"],"quay.io":["quay.example"]}}"#,
    );
    assert!(index.registry_mirrors_problem.is_none());
    assert_eq!(index.registry_mirrors.docker_hub, ["docker.example:5000"]);
    assert_eq!(index.registry_mirrors.ghcr, ["ghcr.example"]);
    assert_eq!(index.registry_mirrors.quay, ["quay.example"]);
}

/// 名单读不懂时只影响加速，别的目录数据照常读；旧的一维列表写法不再支持。
#[test]
fn unreadable_registry_lists_do_not_break_other_catalog_data() {
    for mirrors in [
        serde_json::json!(["mirror.example:5000"]),
        serde_json::json!([1]),
        serde_json::json!({"ghcr.io": ["https://ghcr.example"]}),
        serde_json::json!({"docker.io": "mirror.example"}),
    ] {
        let index = index(
            &serde_json::to_string(&serde_json::json!({"format":1,"registry_mirrors":mirrors}))
                .unwrap(),
        );
        assert!(index.registry_mirrors_problem.is_some(), "{mirrors}");
        assert!(index.registry_mirrors.docker_hub.is_empty());
        assert!(index.registry_mirrors.ghcr.is_empty());
    }
}

#[test]
fn registry_settings_survive_reopening_the_store() {
    let dir = tempfile::tempdir().unwrap();
    let setting = RegistryMirrorUpdate {
        mirror: Some("mirror.example:5443".to_owned()),
    };
    Store::open(dir.path())
        .unwrap()
        .save_registry_mirror(&setting)
        .unwrap();
    let reopened = Store::open(dir.path()).unwrap();
    assert_eq!(reopened.registry_mirror().unwrap(), Some(setting));
    assert!(reopened.load_all().unwrap().is_empty());
    reopened
        .save_registry_mirror(&RegistryMirrorUpdate { mirror: None })
        .unwrap();
    assert_eq!(
        Store::open(dir.path())
            .unwrap()
            .registry_mirror()
            .unwrap()
            .unwrap()
            .mirror,
        None
    );
}

#[test]
fn registry_recommendation_requires_a_real_slow_or_failed_probe() {
    assert!(!recommended(None));
    let mut probe = RegistryProbe {
        probed_at_ms: 1,
        latency_ms: Some(5000),
        problem: None,
    };
    assert!(!recommended(Some(&probe)));
    probe.latency_ms = Some(5001);
    assert!(recommended(Some(&probe)));
    probe.latency_ms = None;
    probe.problem = Some(RegistryProblem {
        reason: RegistryFailure::Unreachable,
        message: "timeout".into(),
    });
    assert!(recommended(Some(&probe)));
}

/// 设置页的 `choices` 只列 Docker Hub 的站；ghcr、quay 的名单不出现在这个选择里。
#[tokio::test]
async fn settings_only_offer_the_docker_hub_stations() {
    let (docker, engine) = crate::plugin::fake_docker::FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let manager = super::test_support::manager(engine, &dir, "apptest");
    std::fs::write(
        dir.path().join("index/index.json"),
        br#"{"format":1,"plugins":[],"registry_mirrors":{
            "docker.io":["docker.example"],"ghcr.io":["ghcr.example"],"quay.io":["quay.example"]}}"#,
    )
    .unwrap();
    let settings = manager
        .registry_mirror_settings(&Languages::default())
        .await
        .unwrap();
    assert_eq!(settings.choices, ["docker.example"]);
    assert!(settings.choices_problem.is_none());
    drop(docker);
}

/// 主人填的 Docker Hub 地址排在最前面，索引里重复的地址只留一次。
#[tokio::test]
async fn the_owner_address_comes_first_and_is_not_repeated() {
    let (docker, engine) = crate::plugin::fake_docker::FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let manager = super::test_support::manager(engine, &dir, "apptest");
    std::fs::write(
        dir.path().join("index/index.json"),
        br#"{"format":1,"plugins":[],"registry_mirrors":{
            "docker.io":["index.example","owner.example"],"ghcr.io":["ghcr.example"]}}"#,
    )
    .unwrap();
    manager
        .set_registry_mirror(
            RegistryMirrorUpdate {
                mirror: Some("owner.example".to_owned()),
            },
            &Languages::default(),
        )
        .await
        .unwrap();
    let mirrors = manager
        .registry_mirrors(&Languages::default())
        .await
        .unwrap();
    assert_eq!(mirrors.docker_hub, ["owner.example", "index.example"]);
    assert_eq!(mirrors.ghcr, ["ghcr.example"]);
    drop(docker);
}

/// 索引里的地址列表读不懂时，设置页拿到的 choices 是空的，但带上一句问题，不假装没有地址。
#[tokio::test]
async fn unreadable_index_mirrors_reach_the_settings_page_as_a_problem() {
    let (docker, engine) = crate::plugin::fake_docker::FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let manager = super::test_support::manager(engine, &dir, "apptest");
    std::fs::write(
        dir.path().join("index/index.json"),
        br#"{"format":1,"plugins":[],"registry_mirrors":[1]}"#,
    )
    .unwrap();
    let settings = manager
        .registry_mirror_settings(&Languages::default())
        .await
        .unwrap();
    assert!(settings.choices.is_empty());
    assert!(settings.choices_problem.is_some());
    drop(docker);
}

/// 空表是读得懂的：没有名单，不报问题。
#[test]
fn an_empty_table_is_readable() {
    let index = index(r#"{"format":1,"registry_mirrors":{}}"#);
    assert!(index.registry_mirrors_problem.is_none());
    assert!(index.registry_mirrors.docker_hub.is_empty());
}

/// 加速开关总控：关掉时索引里有什么名单都不用。
#[tokio::test]
async fn no_stations_while_acceleration_is_off() {
    let (docker, engine) = crate::plugin::fake_docker::FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let manager = super::test_support::manager(engine, &dir, "apptest");
    std::fs::write(
        dir.path().join("index/index.json"),
        br#"{"format":1,"plugins":[],"registry_mirrors":{"docker.io":["docker.example"],"ghcr.io":["ghcr.example"]}}"#,
    )
    .unwrap();
    let mirrors = manager
        .registry_mirrors(&Languages::default())
        .await
        .unwrap();
    assert_eq!(mirrors, Mirrors::default());
    drop(docker);
}

/// 每个仓库只按自己的名单改引用；Docker Hub 的主人地址排在最前面。
#[tokio::test]
async fn every_registry_uses_its_own_list() {
    let (docker, engine) = crate::plugin::fake_docker::FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let manager = super::test_support::manager(engine, &dir, "apptest");
    std::fs::write(
        dir.path().join("index/index.json"),
        br#"{"format":1,"plugins":[],"registry_mirrors":{
            "docker.io":["docker.example"],"ghcr.io":["ghcr.example"],"quay.io":["quay.example"]}}"#,
    )
    .unwrap();
    manager
        .set_registry_mirror(
            RegistryMirrorUpdate {
                mirror: Some("owner.example".to_owned()),
            },
            &Languages::default(),
        )
        .await
        .unwrap();
    let mirrors = manager
        .registry_mirrors(&Languages::default())
        .await
        .unwrap();
    let digest = "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    for (image, stations, repository) in [
        (
            "docker.io/alpine:3",
            &["owner.example", "docker.example"][..],
            "library/alpine",
        ),
        ("ghcr.io/org/app", &["ghcr.example"][..], "org/app"),
        ("quay.io/org/app", &["quay.example"][..], "org/app"),
    ] {
        let expected = stations
            .iter()
            .map(|station| format!("{station}/{repository}@{digest}"))
            .collect::<Vec<_>>();
        assert_eq!(
            accelerated(&format!("{image}@{digest}"), &mirrors),
            expected,
            "{image}"
        );
    }
    drop(docker);
}
