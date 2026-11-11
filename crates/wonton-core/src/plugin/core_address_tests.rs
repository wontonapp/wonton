use std::collections::HashMap;

use ipnet::Ipv4Net;

use super::docker::NetworkReach;
use super::fake_docker::FakeDocker;

#[tokio::test]
async fn two_cores_picking_the_same_subnet_both_create_disjoint_networks() {
    let (docker, first) = FakeDocker::start().await;
    // 两个独立 Engine 才是两个核心：克隆共享进程内的建网络锁，串起来就不是并发了。
    let second = docker.engine().await;
    let held = docker.hold("POST /networks/create");
    let left = tokio::spawn(async move {
        first
            .create_network("left", &HashMap::new(), NetworkReach::Isolated)
            .await
    });
    held.reached().await;
    let right = tokio::spawn(async move {
        second
            .create_network("right", &HashMap::new(), NetworkReach::Isolated)
            .await
    });
    held.reached().await;
    held.release();
    let (left, right) = tokio::join!(left, right);
    left.unwrap().unwrap();
    right.unwrap().unwrap();
    assert_eq!(docker.network_collisions(), 1);
    let subnet = |name| {
        docker.network(name).unwrap()["IPAM"]["Config"][0]["Subnet"]
            .as_str()
            .unwrap()
            .parse::<Ipv4Net>()
            .unwrap()
    };
    let (left, right) = (subnet("left"), subnet("right"));
    assert!(!left.contains(&right.network()) && !right.contains(&left.network()));
}
