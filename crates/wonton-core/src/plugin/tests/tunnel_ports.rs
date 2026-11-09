use std::os::fd::AsFd;

use wonton_protocol::{PortProtocol, PortSpec};

use super::super::manifest::ManifestError;
use super::super::package_files::{PackageFiles, write_archive};
use super::{PackageError, PluginPackage, RenderError, dry_run, langs};

fn source(ports: &str, extra: &str) -> String {
    format!(
        "services:\n  app: {{image: example/app}}\n  media: {{image: example/media}}\n\
         x-wonton:\n  spec: '0.11'\n  id: app.test\n  version: 1.0.0\n  kind: app\n  \
         title: {{en: Test}}\n  description: {{en: Test}}\n  permissions: []\n  \
         tunnel:\n    ports: [{ports}]\n{extra}"
    )
}

fn parse(ports: &str, extra: &str) -> Result<PluginPackage, PackageError> {
    PluginPackage::parse(&source(ports, extra), &langs())
}

#[test]
fn tunnel_port_range_is_exactly_1024_through_65535() {
    for port in [0, 1023] {
        assert!(matches!(
            parse(&format!("{{service: app, port: {port}}}"), ""),
            Err(PackageError::Manifest(ManifestError::Invalid(_)))
        ));
    }
    for port in [1024, 65535] {
        let package = parse(&format!("{{service: app, port: {port}}}"), "").unwrap();
        assert_eq!(
            package.manifest.tunnel.unwrap().ports,
            [PortSpec {
                service: "app".into(),
                port,
                protocol: PortProtocol::Tcp,
            }]
        );
    }
    assert!(matches!(
        parse("{service: app, port: 65536}", ""),
        Err(PackageError::Manifest(ManifestError::Invalid(_)))
    ));
}

#[test]
fn tunnel_ports_have_one_service_per_port_and_protocol() {
    for duplicate in [
        "{service: app, port: 1024}, {service: app, port: 1024}",
        "{service: app, port: 1024}, {service: app, port: 1024, protocol: tcp}",
        "{service: app, port: 1024, protocol: udp}, {service: app, port: 1024, protocol: udp}",
        "{service: app, port: 1024}, {service: media, port: 1024}",
        "{service: app, port: 1024, protocol: udp}, {service: media, port: 1024, protocol: udp}",
    ] {
        assert!(matches!(
            parse(duplicate, ""),
            Err(PackageError::Manifest(ManifestError::Invalid(_)))
        ));
    }
    let package = parse(
        "{service: app, port: 1024}, {service: media, port: 1024, protocol: udp}",
        "",
    )
    .unwrap();
    assert_eq!(
        package.manifest.tunnel.unwrap().ports,
        [
            PortSpec {
                service: "app".into(),
                port: 1024,
                protocol: PortProtocol::Tcp
            },
            PortSpec {
                service: "media".into(),
                port: 1024,
                protocol: PortProtocol::Udp
            },
        ]
    );
}

#[test]
fn only_the_tcp_mcp_endpoint_is_private_to_the_core() {
    let mcp = "  mcp: {service: app, port: 8080, transport: streamable-http}\n";
    for protocol in ["", ", protocol: tcp"] {
        assert!(matches!(
            parse(&format!("{{service: app, port: 8080{protocol}}}"), mcp),
            Err(PackageError::Manifest(ManifestError::Invalid(_)))
        ));
    }
    parse("{service: app, port: 8080, protocol: udp}", mcp).unwrap();
    parse("{service: media, port: 8080}", mcp).unwrap();
    parse("{service: app, port: 8081}", mcp).unwrap();
}

#[test]
fn every_package_entrypoint_rejects_an_unknown_tunnel_service() {
    let source = source("{service: missing, port: 1024}", "");
    let files = PackageFiles::new();
    let archive = write_archive(source.as_bytes(), &files);
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("docker-compose.yml"), &source).unwrap();
    let opened = std::fs::File::open(dir.path()).unwrap();
    let results = [
        PluginPackage::parse(&source, &langs()).map(|_| ()),
        PluginPackage::with_files(&source, files, &langs()).map(|_| ()),
        PluginPackage::manifest_of(&source, &langs()).map(|_| ()),
        PluginPackage::referenced_files(&source, &langs()).map(|_| ()),
        PluginPackage::optional_files(&source, &langs()).map(|_| ()),
        PluginPackage::from_archive(&archive, &langs()).map(|_| ()),
        PluginPackage::load_dir(dir.path(), &langs()).map(|_| ()),
        PluginPackage::load_dir_at(opened.as_fd(), &langs()).map(|_| ()),
    ];
    for result in results {
        assert!(matches!(
            result,
            Err(PackageError::UnknownService { field: "tunnel.ports", service, .. })
                if service == "missing"
        ));
    }
}

#[test]
fn tunnel_declarations_do_not_publish_any_network_port() {
    for protocol in ["tcp", "udp", "future"] {
        let package = parse(
            &format!("{{service: app, port: 1024, protocol: {protocol}}}"),
            "",
        )
        .unwrap();
        let expected = match protocol {
            "tcp" => PortProtocol::Tcp,
            "udp" => PortProtocol::Udp,
            _ => PortProtocol::Other,
        };
        assert_eq!(
            package.manifest.tunnel.as_ref().unwrap().ports[0].protocol,
            expected
        );
        for container in dry_run(&package, &langs()).unwrap().containers {
            assert!(container.body.exposed_ports.is_none());
            assert!(
                container
                    .body
                    .host_config
                    .is_none_or(|host| host.port_bindings.is_none())
            );
        }
    }
}

#[test]
fn unknown_lan_protocol_is_readable_but_never_rendered_as_tcp() {
    let source = source("", "").replace("permissions: []", "permissions: [network.lan]")
        + "  lan: {ports: [{service: app, port: 1024, protocol: future}]}\n";
    let package = PluginPackage::parse(&source, &langs()).unwrap();
    assert_eq!(
        package.manifest.lan.as_ref().unwrap().ports[0].protocol,
        PortProtocol::Other
    );
    assert!(matches!(
        dry_run(&package, &langs()),
        Err(RenderError::Service { service, .. }) if service == "app"
    ));
}

#[test]
fn shared_lan_ports_keep_known_docker_protocols_and_tcp_default() {
    for (declaration, protocol) in [
        ("", "tcp"),
        (", protocol: tcp", "tcp"),
        (", protocol: udp", "udp"),
    ] {
        let source = source("", "").replace("permissions: []", "permissions: [network.lan]")
            + &format!("  lan: {{ports: [{{service: app, port: 1024{declaration}}}]}}\n");
        let package = PluginPackage::parse(&source, &langs()).unwrap();
        let plan = dry_run(&package, &langs()).unwrap();
        let body = &plan
            .containers
            .iter()
            .find(|c| c.service == "app")
            .unwrap()
            .body;
        let key = format!("1024/{protocol}");
        assert!(body.exposed_ports.as_ref().unwrap().contains(&key));
        let bindings = body
            .host_config
            .as_ref()
            .unwrap()
            .port_bindings
            .as_ref()
            .unwrap();
        assert_eq!(bindings.len(), 1);
        let binding = &bindings[&key].as_ref().unwrap()[0];
        assert_eq!(binding.host_port.as_deref(), Some("1024"));
        assert_eq!(binding.host_ip.as_deref(), Some("192.0.2.1"));
    }
}
