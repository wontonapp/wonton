use std::collections::HashMap;

use super::docker::NetworkReach;
use super::fake_docker::FakeDocker;
use super::lan::{Detector, probe::HostNetwork};
use super::network_pool::NetworkPool;

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
