//! 底座：镜像、本次运行专属的子网和 challtestsrv。

use super::*;

const PEBBLE: &str = "ghcr.io/letsencrypt/pebble:2.8.0@sha256:d9080f68f6cb6af8d82134ab26de0aaaf312ac9cba42aecc6d3aede6cb63007b";
const CHALLTESTSRV: &str = "ghcr.io/letsencrypt/pebble-challtestsrv:latest@sha256:c390a96cc878a7d5bf5e27aaf99af532c46c4b38416d72f7219ab9ded51bb79f";
/// 按 amd64 的镜像摘要固定（不是 manifest list 的摘要：编译机对 list 摘要的 HEAD 不稳）。
const PYTHON: &str =
    "python:3.12-alpine@sha256:e6dae589c1c05976188fd3bd019afd091445fc533ed2ea29001beddc7368a9c8";

/// 本次运行专属的子网，和周围的任务不撞。
fn subnet() -> (String, String) {
    let hash = getrandom::u32().unwrap();
    let v4 = format!("10.{}.{}.0/24", 30 + (hash % 150), (hash >> 8) & 0xff);
    (v4.trim_end_matches(".0/24").to_owned(), v4)
}

pub(super) async fn up(case: &mut Case) {
    case.haproxy = ensure_image(&case.engine, &wonton_test_images::image(HAPROXY_IMAGE)).await;
    case.pebble = ensure_image(&case.engine, &wonton_test_images::image(PEBBLE)).await;
    let challtestsrv = ensure_image(&case.engine, &wonton_test_images::image(CHALLTESTSRV)).await;
    let _lego = ensure_image(&case.engine, &wonton_test_images::image(LEGO_IMAGE)).await;
    case.python = ensure_image(&case.engine, &wonton_test_images::image(PYTHON)).await;

    let (base, cidr) = subnet();
    case.base = base;
    let network = case.names.oneshot_network("cert");
    must_docker(&[
        "network",
        "create",
        "--label",
        &format!("{LABEL_INSTANCE}={}", case.names.name()),
        "--subnet",
        &cidr,
        &network,
    ]);
    case.network = network;
    case.challenge_ip = format!("{}.5", case.base);
    case.pebble_ip = format!("{}.6", case.base);
    case.entry_ip = format!("{}.11", case.base);
    case.connector_ip = format!("{}.21", case.base);

    case.chall = case.names.oneshot_container("challtestsrv");
    start(
        &case.names,
        &case.network,
        &case.chall,
        None,
        &case.challenge_ip,
        &[],
        &[],
        &challtestsrv,
        "/app",
        &[
            "-dnsserver",
            ":53",
            "-http01",
            "",
            "-https01",
            "",
            "-tlsalpn01",
            "",
        ],
    );
}
