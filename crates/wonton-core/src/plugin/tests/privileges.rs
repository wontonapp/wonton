//! 清单里的高权限字段：没声明权限就拒绝，声明了才写进容器配置。

use super::*;

#[test]
fn security_sensitive_fields_are_rejected_with_reason() {
    for field in [
        "ports: ['8080:80']",
        "container_name: x",
        "build: .",
        "env_file: [.env]",
    ] {
        let yaml = package(
            &format!("  web:\n    image: nginx\n    {field}"),
            "  permissions: []",
        );
        let err = PluginPackage::parse(&yaml, &langs()).unwrap_err();
        assert!(
            matches!(err, PackageError::Service { .. }),
            "{field}: {err}"
        );
    }
    for (field, manifest_field, permission) in [
        ("privileged: true", "privileged: true", "privileged"),
        ("network_mode: host", "network_mode: host", "network.host"),
        (
            "cap_add: [NET_ADMIN]",
            "cap_add: NET_ADMIN",
            "cap.NET_ADMIN",
        ),
        (
            "devices: ['/dev/net/tun:/dev/tun']",
            "devices: /dev/net/tun",
            "devices./dev/net/tun",
        ),
        ("pid: host", "pid: host", "host.pid"),
        ("ipc: host", "ipc: host", "host.ipc"),
        (
            "security_opt: ['label=disable']",
            "security_opt",
            "security.opt",
        ),
    ] {
        let yaml = package(
            &format!("  web:\n    image: nginx\n    {field}"),
            "  permissions: []",
        );
        let package = PluginPackage::parse(&yaml, &langs()).unwrap();
        let err = dry_run(&package, &langs()).unwrap_err().to_string();
        let reason = message(
            "plugin-install-priv-field-requires",
            &[("field", manifest_field), ("permission", permission)],
        );
        let expected = message(
            "plugin-install-service-error",
            &[("service", "web"), ("error", &reason)],
        );
        assert!(err.contains(&expected), "{field}: {err}");
    }
    let top = format!(
        "networks: {{}}\n{}",
        package("  web:\n    image: nginx", "  permissions: []")
    );
    assert!(matches!(
        PluginPackage::parse(&top, &langs()),
        Err(PackageError::TopLevel { .. })
    ));
}

#[test]
fn host_network_drops_net_raw_unless_declared() {
    let yaml = package(
        "  web:\n    image: nginx\n    network_mode: host",
        "  permissions: [network.host]",
    );
    let plan = dry_run(&PluginPackage::parse(&yaml, &langs()).unwrap(), &langs()).unwrap();
    let host = plan.containers[0].body.host_config.as_ref().unwrap();
    assert_eq!(host.network_mode.as_deref(), Some("host"));
    assert_eq!(host.cap_drop.as_ref().unwrap(), &vec!["NET_RAW".to_owned()]);

    let yaml = package(
        "  web:\n    image: nginx\n    network_mode: host\n    cap_add: [NET_RAW]",
        "  permissions: [network.host, cap.NET_RAW]",
    );
    let plan = dry_run(&PluginPackage::parse(&yaml, &langs()).unwrap(), &langs()).unwrap();
    let host = plan.containers[0].body.host_config.as_ref().unwrap();
    assert_eq!(host.cap_add.as_ref().unwrap(), &vec!["NET_RAW".to_owned()]);
    assert!(host.cap_drop.is_none());
}

#[test]
fn agent_and_bot_manifests_reject_very_high_and_highest_permissions() {
    for permission in ["cap.NET_ADMIN", "privileged"] {
        let high_risk = message(
            "plugin-install-priv-agent-high-risk",
            &[("permission", permission)],
        );
        let agent = package(
            "  web:\n    image: nginx",
            &format!("  permissions: [{permission}]"),
        )
        .replace("  kind: app\n", "  kind: agent\n  agent: {protocol: acp}\n");
        let err = PluginPackage::parse(&agent, &langs())
            .unwrap_err()
            .to_string();
        assert!(err.contains(&high_risk), "{permission}: {err}");

        let bot = package(
            "  web:\n    image: nginx",
            &format!("  permissions: [matrix.bot, {permission}]"),
        );
        let err = PluginPackage::parse(&bot, &langs())
            .unwrap_err()
            .to_string();
        assert!(err.contains(&high_risk), "{permission}: {err}");

        let bots = package(
            "  web:\n    image: nginx",
            &format!("  bots: {{services: [web]}}\n  permissions: [matrix.bot, {permission}]"),
        );
        let err = PluginPackage::parse(&bots, &langs())
            .unwrap_err()
            .to_string();
        assert!(err.contains(&high_risk), "{permission}: {err}");
    }
}

#[test]
fn fixed_devices_need_permission_and_reach_host_config() {
    let missing = package(
        "  web:\n    image: nginx\n    devices: ['/dev/net/tun:/dev/net/tun']",
        "  permissions: []",
    );
    assert!(render_err(&missing).contains("devices./dev/net/tun"));

    let granted = package(
        "  web:\n    image: nginx\n    devices: ['/dev/net/tun:/dev/net/tun']",
        "  permissions: [devices./dev/net/tun]",
    );
    let plan = dry_run(&PluginPackage::parse(&granted, &langs()).unwrap(), &langs()).unwrap();
    let devices = plan.containers[0]
        .body
        .host_config
        .as_ref()
        .unwrap()
        .devices
        .as_ref()
        .unwrap();
    assert_eq!(devices.len(), 1);
    assert_eq!(devices[0].path_on_host.as_deref(), Some("/dev/net/tun"));
    assert_eq!(
        devices[0].path_in_container.as_deref(),
        Some("/dev/net/tun")
    );
    assert_eq!(devices[0].cgroup_permissions.as_deref(), Some("rwm"));

    let unknown = package(
        "  web:\n    image: nginx\n    devices: ['/dev/sda:/dev/sda']",
        "  permissions: [devices./dev/sda]",
    );
    let err = PluginPackage::parse(&unknown, &langs())
        .unwrap_err()
        .to_string();
    assert!(
        err.contains(&message(
            "plugin-install-priv-fixed-device",
            &[("permission", "/dev/sda")]
        )),
        "{err}"
    );
}

#[test]
fn docker_socket_requires_exact_writable_mount() {
    let compose = |volume: &str, permissions: &str| {
        package(
            &format!("  web:\n    image: nginx\n    volumes: ['{volume}']"),
            &format!("  permissions: [{permissions}]"),
        )
    };
    let missing = compose("/var/run/docker.sock:/var/run/docker.sock", "");
    assert!(render_err(&missing).contains("docker.socket"));

    let granted = compose("/var/run/docker.sock:/var/run/docker.sock", "docker.socket");
    let plan = dry_run(&PluginPackage::parse(&granted, &langs()).unwrap(), &langs()).unwrap();
    let mount = &plan.containers[0]
        .body
        .host_config
        .as_ref()
        .unwrap()
        .mounts
        .as_ref()
        .unwrap()[0];
    assert_eq!(mount.source.as_deref(), Some("/var/run/docker.sock"));
    assert_eq!(mount.target.as_deref(), Some("/var/run/docker.sock"));
    assert_eq!(mount.read_only, Some(false));

    for volume in [
        "/tmp/docker.sock:/var/run/docker.sock",
        "/var/run/docker.sock:/other/docker.sock",
        "/var/run/docker.sock:/var/run/docker.sock:ro",
    ] {
        let err = render_err(&compose(volume, "docker.socket"));
        assert!(err.contains("Docker socket"), "{volume}: {err}");
    }
}

#[test]
fn host_network_cannot_share_lan_ports() {
    let yaml = package(
        "  web:\n    image: nginx\n    network_mode: host",
        "  lan: {ports: [{service: web, port: 8080}]}\n  permissions: [network.host, network.lan]",
    );
    let err = render_err(&yaml);
    assert!(
        err.contains(&message("plugin-install-priv-host-network-ports", &[])),
        "{err}"
    );
}

#[test]
fn host_webui_requires_healthcheck() {
    let missing = package(
        "  web:\n    image: nginx\n    network_mode: host",
        "  webui: {service: web, port: 80, sso: none}\n  permissions: [network.host]",
    );
    assert!(render_err(&missing).contains(&message(
        "plugin-install-priv-host-network-healthcheck",
        &[]
    )));

    let valid = package(
        "  web:\n    image: nginx\n    network_mode: host\n    healthcheck: {test: ['CMD', 'true']}",
        "  webui: {service: web, port: 80, sso: none}\n  permissions: [network.host]",
    );
    dry_run(&PluginPackage::parse(&valid, &langs()).unwrap(), &langs()).unwrap();
}

#[test]
fn host_webui_cannot_use_proxy_sso() {
    let yaml = package(
        "  web:\n    image: nginx\n    network_mode: host\n    healthcheck: {test: ['CMD', 'true']}",
        "  webui:\n    service: web\n    port: 80\n    sso: proxy\n    proxy:\n      headers: {Remote-User: '{user}'}\n  permissions: [network.host]",
    );
    let err = render_err(&yaml);
    assert!(err.contains("sso: proxy"), "{err}");
}

#[test]
fn unknown_capabilities_are_highest_risk() {
    use crate::plugin::privileges::{CapabilityRisk, capability_risk};

    assert_eq!(capability_risk("NET_ADMIN"), CapabilityRisk::VeryHigh);
    assert_eq!(capability_risk("NET_RAW"), CapabilityRisk::VeryHigh);
    assert_eq!(capability_risk("SYS_ADMIN"), CapabilityRisk::Highest);
    assert_eq!(
        capability_risk("future_kernel_cap"),
        CapabilityRisk::Highest
    );
}

#[test]
fn app_provides_is_rejected() {
    let yaml = package(
        "  web:\n    image: nginx",
        "  provides:\n    models:\n      service: web\n      port: 8080\n      apis: {openai: /v1}\n      default_model: test\n      credentials:\n        issue: [issue, '{name}']\n        revoke: [revoke, '{name}']\n  permissions: []",
    );
    let err = PluginPackage::parse(&yaml, &langs()).unwrap_err();
    assert_eq!(
        err.to_string(),
        message("plugin-install-prv-models-system-only", &[]),
        "{err}"
    );
}

#[test]
fn unknown_fields_fail_but_extensions_pass() {
    let unknown = package(
        "  web:\n    image: nginx\n    frobnicate: 1",
        "  permissions: []",
    );
    assert!(render_err(&unknown).contains("frobnicate"));
    let include = format!(
        "include: [a.yml]\n{}",
        package("  web:\n    image: nginx", "  permissions: []")
    );
    assert!(PluginPackage::parse(&include, &langs()).is_err());
    let ext = format!(
        "x-casaos: {{main: web}}\n{}",
        package(
            "  web:\n    image: nginx\n    x-casaos: {envs: []}",
            "  permissions: []"
        )
    );
    dry_run(&PluginPackage::parse(&ext, &langs()).unwrap(), &langs()).unwrap();
}

#[test]
fn compose_without_manifest_is_refused() {
    let err = PluginPackage::parse("services:\n  web:\n    image: nginx\n", &langs()).unwrap_err();
    assert!(matches!(err, PackageError::MissingManifest(_)));
}
