use std::collections::HashMap;

use super::docker::NetworkReach;
use super::fake_docker::FakeDocker;
use super::lan::{Detector, probe::HostNetwork};
use super::network_pool::NetworkPool;

/// 把换过镜像的说明拆成前后两段（`reason` 参数的位置用记号占着），断言用。
fn replaced_note(langs: &crate::plugin::languages::Languages) -> (String, String) {
    let note = crate::l10n::text(
        langs,
        "plugin-install-probe-image-replaced",
        &[("reason", String::from("«REASON»").into())],
    );
    let (prefix, suffix) = note.split_once("«REASON»").expect("说明里有 reason 参数");
    (prefix.to_owned(), suffix.to_owned())
}

#[tokio::test]
async fn containerized_core_reads_host_routes_with_the_existing_one_shot_probe() {
    let (docker, engine) = FakeDocker::start().await;
    docker.add_image_container("core", "our-core-image");
    let output = serde_json::to_string(&HostNetwork {
        subnets: vec!["10.213.0.0/24".parse().unwrap()],
        ..Default::default()
    })
    .unwrap();
    docker.set_output("core-route-probe", 0, &output, "");
    let engine = engine.with_network_pool(
        NetworkPool::default(),
        Detector::Container {
            core: "core".into(),
        },
    );
    engine
        .create_network("plugin", &HashMap::new(), NetworkReach::Isolated)
        .await
        .unwrap();
    let chosen = docker.network("plugin").unwrap();
    assert_eq!(chosen["IPAM"]["Config"][0]["Subnet"], "10.213.1.0/26");
    assert_eq!(docker.containers(), ["core"]);
}

#[tokio::test]
async fn failed_host_probe_is_cleaned_and_no_network_is_created() {
    let (docker, engine) = FakeDocker::start().await;
    docker.add_image_container("core", "our-core-image");
    docker.set_output("core-route-probe", 1, "", "cannot read host routes");
    let engine = engine.with_network_pool(
        NetworkPool::default(),
        Detector::Container {
            core: "core".into(),
        },
    );
    assert!(
        engine
            .create_network("plugin", &HashMap::new(), NetworkReach::Isolated)
            .await
            .is_err()
    );
    assert!(docker.networks().is_empty());
    assert_eq!(docker.containers(), ["core"]);
}

/// 核心还在跑旧镜像 ID，创建时的引用已指向新镜像：按 ID 起不了（containerd 存储下镜像记录被清掉，#666），
/// 退回按引用起，建网络照常完成。
#[tokio::test]
async fn a_replaced_core_image_falls_back_to_the_creation_reference() {
    let (docker, engine) = FakeDocker::start().await;
    docker.add_core_container("core", "sha256:old-core", "wonton-core:dev");
    docker.forget_image("sha256:old-core");
    let output = serde_json::to_string(&HostNetwork {
        subnets: vec!["10.213.0.0/24".parse().unwrap()],
        ..Default::default()
    })
    .unwrap();
    docker.set_output("core-route-probe", 0, &output, "");
    let engine = engine.with_network_pool(
        NetworkPool::default(),
        Detector::Container {
            core: "core".into(),
        },
    );
    engine
        .create_network("plugin", &HashMap::new(), NetworkReach::Isolated)
        .await
        .unwrap();
    let created = docker.created("core-route-probe").unwrap();
    assert_eq!(created["Image"], "wonton-core:dev");
    assert_eq!(docker.networks(), ["plugin"]);
    assert_eq!(docker.containers(), ["core"]);
}

/// 引用也查不到（标签被删了）：如实报错，带上 Docker 的原话，探测容器不留残骸。
#[tokio::test]
async fn a_core_image_that_is_gone_entirely_fails_with_dockers_words() {
    let (docker, engine) = FakeDocker::start().await;
    docker.add_core_container("core", "sha256:old-core", "wonton-core:dev");
    docker.forget_image("sha256:old-core");
    docker.forget_image("wonton-core:dev");
    let engine = engine.with_network_pool(
        NetworkPool::default(),
        Detector::Container {
            core: "core".into(),
        },
    );
    let err = engine
        .create_network("plugin", &HashMap::new(), NetworkReach::Isolated)
        .await
        .unwrap_err();
    assert!(err.to_string().contains("No such image"), "{err}");
    assert!(docker.networks().is_empty());
    assert_eq!(docker.containers(), ["core"]);
}

/// 换用的镜像起得来但探测跑不成（新版本没有这个子命令）：失败原因里说明镜像已被替换、重启就恢复。
#[tokio::test]
async fn a_probe_that_still_fails_on_the_replaced_image_says_so() {
    let (docker, engine) = FakeDocker::start().await;
    docker.add_core_container("core", "sha256:old-core", "wonton-core:dev");
    docker.forget_image("sha256:old-core");
    docker.set_output("core-route-probe", 127, "", "lan-probe: not found");
    let engine = engine.with_network_pool(
        NetworkPool::default(),
        Detector::Container {
            core: "core".into(),
        },
    );
    let err = engine
        .create_network("plugin", &HashMap::new(), NetworkReach::Isolated)
        .await
        .unwrap_err();
    let langs = engine.owner_languages();
    let (prefix, suffix) = replaced_note(&langs);
    let reason = crate::l10n::text(
        &langs,
        "plugin-settings-lan-detect-exit",
        &[
            ("code", String::from("127").into()),
            ("reason", String::from("lan-probe: not found").into()),
        ],
    );
    let message = err.to_string();
    assert!(
        message.contains(&format!("{prefix}{reason}{suffix}")),
        "{message}"
    );
    assert_eq!(docker.containers(), ["core"]);
}

/// 换用的镜像跑出来的输出读不了（新旧版本的输出对不上）：同样如实说明镜像已被替换。
#[tokio::test]
async fn unreadable_output_from_the_replaced_image_says_the_image_was_replaced() {
    let (docker, engine) = FakeDocker::start().await;
    docker.add_core_container("core", "sha256:old-core", "wonton-core:dev");
    docker.forget_image("sha256:old-core");
    docker.set_output("core-route-probe", 0, "not json", "");
    let detected = Detector::Container {
        core: "core".into(),
    }
    .detect(&engine, "core-route-probe", HashMap::new())
    .await
    .unwrap();
    let langs = engine.owner_languages();
    let (prefix, suffix) = replaced_note(&langs);
    let reason = detected.unwrap_err();
    let unreadable = crate::l10n::text(
        &langs,
        "plugin-settings-lan-detect-output",
        &[("error", String::from("«ERROR»").into())],
    );
    let (unreadable, _) = unreadable
        .split_once("«ERROR»")
        .expect("说明里有 error 参数");
    assert!(reason.contains(unreadable), "{reason}");
    assert!(
        reason.starts_with(&prefix) && reason.ends_with(&suffix),
        "{reason}"
    );
}
