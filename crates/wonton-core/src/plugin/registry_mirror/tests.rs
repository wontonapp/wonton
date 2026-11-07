use super::*;
use crate::plugin::languages::Languages;
use crate::plugin::store::Store;
use crate::plugin::storefront::index::Index;
use wonton_protocol::{RegistryFailure, RegistryProblem};

#[test]
fn registry_index_bad_choices_do_not_break_other_catalog_data() {
    let index = Index::parse(
        br#"{"format":1,"plugins":[],"registry_mirrors":["mirror.example:5000"]}"#,
        &Languages::default(),
    )
    .unwrap();
    assert_eq!(index.registry_mirrors, ["mirror.example:5000"]);
    for choices in [
        serde_json::json!([1]),
        serde_json::json!({}),
        serde_json::json!(["https://mirror.example"]),
    ] {
        let index = Index::parse(
            &serde_json::to_vec(&serde_json::json!({"format":1,"registry_mirrors":choices}))
                .unwrap(),
            &Languages::default(),
        )
        .unwrap();
        assert!(index.registry_mirrors_problem.is_some());
        assert!(index.registry_mirrors.is_empty());
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
