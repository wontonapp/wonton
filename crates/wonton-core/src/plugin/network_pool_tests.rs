use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use ipnet::Ipv4Net;
use serde_json::json;

use super::docker::{Engine, EngineError, NetworkReach};
use super::fake_docker::FakeDocker;
use super::lan::{Detector, probe::HostNetwork};
use super::manager::InstallError;
use super::network_pool::{NetworkPool, overlaps};

fn fixed(host: Arc<Mutex<Result<HostNetwork, String>>>) -> Detector {
    Detector::Fixed(host)
}

fn subnet(docker: &FakeDocker, name: &str) -> Ipv4Net {
    docker.network(name).unwrap()["IPAM"]["Config"][0]["Subnet"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap()
}

async fn create(engine: &Engine, name: &str) {
    engine
        .create_network(name, &HashMap::new(), NetworkReach::Isolated)
        .await
        .unwrap();
}

#[tokio::test]
async fn host_routes_are_read_again_and_overlap_of_any_size_is_skipped() {
    let (docker, engine) = FakeDocker::start().await;
    let host = Arc::new(Mutex::new(Ok(HostNetwork {
        subnets: vec!["10.213.0.0/24".parse().unwrap()],
        ..Default::default()
    })));
    let engine = engine.with_network_pool(NetworkPool::default(), fixed(host.clone()));
    create(&engine, "first").await;
    assert_eq!(subnet(&docker, "first"), "10.213.1.0/26".parse().unwrap());
    host.lock()
        .unwrap()
        .as_mut()
        .unwrap()
        .subnets
        .push("10.213.1.65/32".parse().unwrap());
    create(&engine, "second").await;
    assert_eq!(
        subnet(&docker, "second"),
        "10.213.1.128/26".parse().unwrap()
    );
}

#[tokio::test]
async fn all_docker_ipam_subnets_are_skipped_even_without_core_labels() {
    let (docker, engine) = FakeDocker::start().await;
    docker.add_network(
        "other",
        "not-ours",
        json!({
            "IPAM": {"Config": [
                {"Subnet": "10.213.0.0/26"},
                {"Subnet": "10.213.0.64/26"},
                {"Subnet": "fd00::/64"}
            ]}
        }),
    );
    create(&engine, "plugin").await;
    assert_eq!(
        subnet(&docker, "plugin"),
        "10.213.0.128/26".parse().unwrap()
    );
}

#[tokio::test]
async fn configured_pool_is_shared_by_plugin_lan_and_relay_networks() {
    let (docker, engine) = FakeDocker::start().await;
    let pool: NetworkPool = "10.214.0.0/24".parse().unwrap();
    let engine = engine.with_network_pool(
        pool,
        fixed(Arc::new(Mutex::new(Ok(HostNetwork::default())))),
    );
    engine
        .create_network("plugin", &HashMap::new(), NetworkReach::Internal)
        .await
        .unwrap();
    engine
        .create_lan_network("lan", &HashMap::new())
        .await
        .unwrap();
    engine
        .create_relay_network("relay", &HashMap::new())
        .await
        .unwrap();
    let networks = [
        subnet(&docker, "plugin"),
        subnet(&docker, "lan"),
        subnet(&docker, "relay"),
    ];
    for (index, network) in networks.iter().enumerate() {
        assert_eq!(network.prefix_len(), 26);
        assert!(pool.subnet().contains(network));
        for other in &networks[..index] {
            assert!(!overlaps(*network, *other));
        }
        assert_eq!(
            engine
                .core_address(["plugin", "lan", "relay"][index])
                .await
                .unwrap()
                .to_bits(),
            network.broadcast().to_bits() - 1
        );
    }
}

#[tokio::test]
async fn an_exhausted_pool_reports_existing_error_category_without_create_or_retry() {
    let (docker, engine) = FakeDocker::start().await;
    let pool: NetworkPool = "10.214.0.0/26".parse().unwrap();
    let engine = engine.with_network_pool(
        pool,
        fixed(Arc::new(Mutex::new(Ok(HostNetwork::default())))),
    );
    create(&engine, "first").await;
    let before = docker.writes();
    let error = engine
        .create_network("second", &HashMap::new(), NetworkReach::Isolated)
        .await
        .unwrap_err();
    assert!(matches!(error, EngineError::Rejected { .. }));
    assert_eq!(InstallError::from(error).kind(), "docker_rejected");
    assert_eq!(docker.writes(), before);
    assert_eq!(docker.networks(), ["first"]);
}

#[tokio::test]
async fn unavailable_host_routes_stop_network_creation() {
    let (docker, engine) = FakeDocker::start().await;
    let engine = engine.with_network_pool(
        NetworkPool::default(),
        fixed(Arc::new(Mutex::new(Err("probe failed".into())))),
    );
    assert!(matches!(
        engine
            .create_network("plugin", &HashMap::new(), NetworkReach::Isolated)
            .await,
        Err(EngineError::Rejected { .. })
    ));
    assert!(docker.networks().is_empty());
}

#[test]
fn configuration_rejects_nonprivate_noncanonical_or_too_small_pools() {
    for invalid in [
        "0.0.0.0/0",
        "10.0.0.0/7",
        "10.213.0.1/16",
        "10.213.0.0/27",
        "192.0.2.0/24",
        "fd00::/64",
        "nonsense",
    ] {
        assert!(invalid.parse::<NetworkPool>().is_err(), "{invalid}");
    }
    for valid in [
        "10.0.0.0/8",
        "172.16.0.0/12",
        "192.168.0.0/16",
        "10.214.0.0/26",
    ] {
        assert_eq!(
            valid.parse::<NetworkPool>().unwrap().subnet().to_string(),
            valid
        );
    }
}
