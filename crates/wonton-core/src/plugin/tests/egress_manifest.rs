use wonton_protocol::PermissionRisk;

use super::super::languages::Languages;
use super::super::manifest::{Permission, PermissionExt, PluginKind};
use super::super::package::{PackageError, PluginPackage};
use super::super::views_permissions::permission_line;
use super::package;

const SERVICES: &str = "  proxy:\n    image: proxy\n  worker:\n    image: worker";
const PROVIDER: &str = "  provides:\n    egress_proxy: {service: proxy, port: 7890}";

fn source(permissions: &str, provides: &str) -> String {
    package(
        SERVICES,
        &format!("  permissions: [{permissions}]\n{provides}"),
    )
}

#[test]
fn egress_manifest_app_can_provide_proxy() {
    let package = PluginPackage::parse(
        &source("network.egress, egress.proxy", PROVIDER),
        &super::langs(),
    )
    .unwrap();
    assert_eq!(package.manifest.kind, PluginKind::App);
    let provider = package.manifest.provides.unwrap().egress_proxy.unwrap();
    assert_eq!(provider.service, "proxy");
    assert_eq!(provider.port, 7890);
}

#[test]
fn egress_manifest_system_and_agent_can_provide_proxy() {
    let source = source("network.egress, egress.proxy", PROVIDER);
    PluginPackage::parse(
        &source.replace("kind: app", "kind: system"),
        &super::langs(),
    )
    .unwrap();
    let agent = source.replace("kind: app", "kind: agent\n  agent: {protocol: acp}");
    PluginPackage::parse(&agent, &super::langs()).unwrap();
}

#[test]
fn egress_manifest_provider_requires_proxy_permission() {
    let error =
        PluginPackage::parse(&source("network.egress", PROVIDER), &super::langs()).unwrap_err();
    assert!(error.to_string().contains("egress.proxy"));
}

#[test]
fn egress_manifest_permission_requires_provider() {
    for provides in ["", "  provides: {}"] {
        let error = PluginPackage::parse(
            &source("network.egress, egress.proxy", provides),
            &super::langs(),
        )
        .unwrap_err();
        assert!(error.to_string().contains("provides.egress_proxy"));
    }
}

#[test]
fn egress_manifest_requires_network_egress() {
    let error =
        PluginPackage::parse(&source("egress.proxy", PROVIDER), &super::langs()).unwrap_err();
    assert!(error.to_string().contains("network.egress"));
}

#[test]
fn egress_manifest_port_cannot_be_zero() {
    let source = source("network.egress, egress.proxy", PROVIDER).replace("7890", "0");
    let error = PluginPackage::parse(&source, &super::langs()).unwrap_err();
    assert!(error.to_string().contains("provides.egress_proxy.port"));
}

#[test]
fn egress_manifest_requires_service_and_port_fields() {
    for (declaration, missing) in [("{port: 7890}", "service"), ("{service: proxy}", "port")] {
        let source = source(
            "network.egress, egress.proxy",
            &format!("  provides:\n    egress_proxy: {declaration}"),
        );
        let error = PluginPackage::parse(&source, &super::langs()).unwrap_err();
        assert!(error.to_string().contains(missing));
    }
}

#[test]
fn egress_manifest_requires_existing_compose_service() {
    let source = source("network.egress, egress.proxy", PROVIDER)
        .replace("service: proxy", "service: missing");
    let error = PluginPackage::parse(&source, &super::langs()).unwrap_err();
    assert!(matches!(
        error,
        PackageError::UnknownService {
            field,
            service,
            ..
        } if field == "provides.egress_proxy.service" && service == "missing"
    ));
}

#[test]
fn egress_manifest_provider_cannot_set_any_network_mode() {
    for mode in ["none", "host", "null", "${NETWORK_MODE}"] {
        let source = source("network.egress, egress.proxy", PROVIDER).replace(
            "image: proxy",
            &format!("image: proxy\n    network_mode: {mode}"),
        );
        let error = PluginPackage::parse(&source, &super::langs()).unwrap_err();
        assert!(matches!(error, PackageError::Service { service, .. } if service == "proxy"));
    }
}

#[test]
fn egress_manifest_other_services_can_set_network_mode() {
    let source = source("network.egress, egress.proxy", PROVIDER)
        .replace("image: worker", "image: worker\n    network_mode: none");
    PluginPackage::parse(&source, &super::langs()).unwrap();
}

#[test]
fn egress_manifest_provider_cannot_be_a_bot_service() {
    let source = source("network.egress, egress.proxy, matrix.bot", PROVIDER);
    let source = format!("{source}\n  bots: {{services: [proxy]}}");
    let error = PluginPackage::parse(&source, &super::langs()).unwrap_err();
    assert!(matches!(error, PackageError::Service { service, .. } if service == "proxy"));
}

#[test]
fn egress_manifest_plugin_can_have_other_bot_services() {
    let source = source("network.egress, egress.proxy, matrix.bot", PROVIDER);
    let source = format!("{source}\n  bots: {{services: [worker]}}");
    PluginPackage::parse(&source, &super::langs()).unwrap();
}

#[test]
fn egress_manifest_models_remain_system_only() {
    let models = "  provides:\n    models:\n      service: proxy\n      port: 8080\n      \
        apis: {openai: /v1}\n      default_model: default\n      credentials:\n        \
        issue: [issue, '{name}']\n        revoke: [revoke, '{name}']";
    let source = source("", models);
    assert!(PluginPackage::parse(&source, &super::langs()).is_err());
    PluginPackage::parse(
        &source.replace("kind: app", "kind: system"),
        &super::langs(),
    )
    .unwrap();
}

#[test]
fn egress_manifest_permission_round_trips_and_previews_high_risk() {
    let permission: Permission = serde_json::from_str("\"egress.proxy\"").unwrap();
    assert_eq!(permission, Permission::EgressProxy);
    assert_eq!(
        serde_json::to_string(&permission).unwrap(),
        "\"egress.proxy\""
    );
    assert!(permission.is_high());
    for language in ["zh", "en"] {
        let line = permission_line(&permission, &Languages::from_header(Some(language)));
        assert_eq!(line.permission, "egress.proxy");
        assert_eq!(line.risk, Some(PermissionRisk::High));
        assert!(line.notice.is_some());
    }
}
