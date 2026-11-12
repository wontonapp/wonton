//! 清单里 `provides.<能力>` 的契约测试：服务名要和 compose 对得上，跑凭据命令的服务也是，
//! 且都不能是 `network_mode: none` 的（核心要转发进去，ADR-0053）。

use super::super::package::PluginPackage;
use super::package;

const SERVICES: &str = "  qbittorrent:\n    image: qbittorrent\n  jellyfin:\n    image: jellyfin\n  issuer:\n    image: issuer";

fn source(provides: &str) -> String {
    package(
        SERVICES,
        &format!("  permissions: []\n  provides:\n{provides}"),
    )
}

#[test]
fn capability_provider_service_must_exist_in_compose() {
    let source = source("    torrent_client: {service: missing, port: 8080}");
    let text = PluginPackage::parse(&source, &super::langs())
        .unwrap_err()
        .to_string();
    assert!(
        text.contains("provides.torrent_client.service") && text.contains("`missing`"),
        "实际错误：{text}"
    );
}

#[test]
fn capability_credentials_service_must_exist_in_compose() {
    let source = source(
        "    jellyfin:\n      service: jellyfin\n      port: 8096\n      credentials:\n        issue: [issue, \"{name}\"]\n        revoke: [revoke, \"{name}\"]\n        service: missing",
    );
    let text = PluginPackage::parse(&source, &super::langs())
        .unwrap_err()
        .to_string();
    assert!(
        text.contains("provides.jellyfin.credentials.service") && text.contains("`missing`"),
        "实际错误：{text}"
    );
}

#[test]
fn capability_provider_service_cannot_be_network_mode_none() {
    let source = source("    torrent_client: {service: qbittorrent, port: 8080}").replace(
        "  qbittorrent:\n    image: qbittorrent",
        "  qbittorrent:\n    image: qbittorrent\n    network_mode: none",
    );
    let text = PluginPackage::parse(&source, &super::langs())
        .unwrap_err()
        .to_string();
    assert!(
        text.contains("provides.torrent_client.service") && text.contains("network_mode: none"),
        "实际错误：{text}"
    );
}

#[test]
fn capability_credentials_service_cannot_be_network_mode_none() {
    let source = source(
        "    jellyfin:\n      service: jellyfin\n      port: 8096\n      credentials:\n        issue: [issue, \"{name}\"]\n        revoke: [revoke, \"{name}\"]\n        service: issuer",
    )
    .replace(
        "  issuer:\n    image: issuer",
        "  issuer:\n    image: issuer\n    network_mode: none",
    );
    let text = PluginPackage::parse(&source, &super::langs())
        .unwrap_err()
        .to_string();
    assert!(
        text.contains("provides.jellyfin.credentials.service")
            && text.contains("network_mode: none"),
        "实际错误：{text}"
    );
}

#[test]
fn capability_provider_with_valid_services_parses() {
    let source = source(
        "    jellyfin:\n      service: jellyfin\n      port: 8096\n      credentials:\n        issue: [issue, \"{name}\"]\n        revoke: [revoke, \"{name}\"]\n        service: issuer",
    );
    PluginPackage::parse(&source, &super::langs()).unwrap();
}

fn provider_manifest(dependency: &str, provides: &str) -> String {
    package(
        SERVICES,
        &format!("  permissions: []\n{dependency}  provides:\n{provides}"),
    )
}

/// ADR-0061 第 9 节：提供 agent_acp 的插件可以 requires/wants 同一个能力（按名字比，不写死某个能力）。
#[test]
fn a_provider_may_declare_the_capability_it_provides() {
    let provides = "    agent_acp: {service: qbittorrent, port: 8080}";
    PluginPackage::parse(
        &provider_manifest("  wants: [agent_acp]\n", provides),
        &super::langs(),
    )
    .unwrap();
    PluginPackage::parse(
        &provider_manifest("  requires: [agent_acp]\n", provides),
        &super::langs(),
    )
    .unwrap();
    let source = provider_manifest(
        "  requires: [torrent_client]\n",
        "    torrent_client: {service: qbittorrent, port: 8080}",
    );
    PluginPackage::parse(&source, &super::langs()).unwrap();
}

#[test]
fn a_provider_still_cannot_require_a_capability_it_does_not_provide() {
    let source = provider_manifest(
        "  requires: [jellyfin]\n",
        "    torrent_client: {service: qbittorrent, port: 8080}",
    );
    let text = PluginPackage::parse(&source, &super::langs())
        .unwrap_err()
        .to_string();
    assert!(text.contains("jellyfin"), "实际错误：{text}");
}
