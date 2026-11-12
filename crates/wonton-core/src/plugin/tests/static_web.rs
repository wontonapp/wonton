//! 静态网页和外部网页两种形态的清单契约（ADR-0080）：字段怎么写、哪些字段不能写、安全档位怎么算，
//! 以及托管能力的 `provides.<能力>.mount`。托管本身没有实现，装的时候如实报“不支持”。

use serde_json::{Value, json};
use wonton_protocol::{PluginRuntime, SecurityTier};

use super::super::manifest::{Manifest, Runtime};
use super::super::package::{PackageError, PluginPackage};
use super::super::security_tier::{prominent, security_tier};
use super::super::spec_floor;

fn base(runtime: &str) -> Value {
    json!({"x-wonton": {
        "spec": "0.12", "id": "io.github.alice.snake", "version": "1.0.0", "kind": "app",
        "runtime": runtime, "title": {"en": "Snake"}, "description": {"en": "Snake"},
        "permissions": []
    }})
}

fn static_web(extra: Value) -> Value {
    let mut doc = base("static_web");
    for (key, value) in extra.as_object().unwrap() {
        doc["x-wonton"][key] = value.clone();
    }
    doc
}

fn link(url: &str) -> Value {
    let mut doc = base("link");
    doc["x-wonton"]["link"] = json!({"url": url});
    doc
}

fn parse(doc: &Value) -> Result<PluginPackage, PackageError> {
    PluginPackage::parse(&doc.to_string(), &super::langs())
}

fn manifest(doc: &Value) -> Manifest {
    parse(doc).unwrap().manifest
}

fn rejected(doc: &Value) -> String {
    match parse(doc) {
        Err(PackageError::Manifest(error)) => error.to_string(),
        other => panic!("应当是清单错误：{other:?}"),
    }
}

#[test]
fn static_web_defaults_its_root_and_maps_to_the_protocol_runtime() {
    let parsed = manifest(&base("static_web"));
    assert_eq!(parsed.runtime, Runtime::StaticWeb);
    assert_eq!(
        PluginRuntime::from(parsed.runtime),
        PluginRuntime::StaticWeb
    );
    assert!(parsed.static_web.is_none());

    let parsed = manifest(&static_web(json!({
        "permissions": ["network.egress"],
        "static_web": {
            "root": "dist/app", "fallback": "index.html",
            "connect": ["https://api.open-meteo.com", "wss://*.example.com"]
        }
    })));
    let spec = parsed.static_web.unwrap();
    assert_eq!(spec.root, "dist/app");
    assert_eq!(spec.fallback.as_deref(), Some("index.html"));
    assert_eq!(spec.connect.len(), 2);
}

#[test]
fn static_web_paths_stay_inside_the_package_and_off_the_gateway_prefix() {
    for root in [
        "",
        "/web",
        "../web",
        "web/../x",
        ".hidden",
        "web/_wonton",
        "a\\b",
    ] {
        let doc = static_web(json!({"static_web": {"root": root}}));
        assert!(rejected(&doc).contains("static_web.root"), "{root}");
    }
    let doc = static_web(json!({"static_web": {"fallback": "../index.html"}}));
    assert!(rejected(&doc).contains("static_web.fallback"));
}

#[test]
fn connect_takes_only_https_or_wss_host_names() {
    for origin in [
        "https://api.example.com",
        "https://*.tile.openstreetmap.org",
        "wss://push.example.com",
    ] {
        let doc = static_web(json!({
            "permissions": ["network.egress"], "static_web": {"connect": [origin]}
        }));
        assert!(parse(&doc).is_ok(), "{origin}");
    }
    for origin in [
        "http://api.example.com",
        "https://api.example.com/v1",
        "https://api.example.com:8443",
        "https://192.0.2.10",
        "https://[2001:db8::1]",
        "https://localhost",
        "https://*",
        "*.example.com",
        "https://API.example.com",
        "https://-bad.example.com",
    ] {
        let doc = static_web(json!({
            "permissions": ["network.egress"], "static_web": {"connect": [origin]}
        }));
        assert!(rejected(&doc).contains("static_web.connect"), "{origin}");
    }
    let many: Vec<String> = (0..33)
        .map(|i| format!("https://h{i}.example.com"))
        .collect();
    let doc = static_web(json!({
        "permissions": ["network.egress"], "static_web": {"connect": many}
    }));
    assert!(rejected(&doc).contains("32"));
}

#[test]
fn connect_and_network_egress_come_together() {
    let without_permission = static_web(json!({
        "static_web": {"connect": ["https://api.example.com"]}
    }));
    let without_connect = static_web(json!({"permissions": ["network.egress"]}));
    for doc in [without_permission, without_connect] {
        assert!(rejected(&doc).contains("network.egress"));
    }
    let other_permission = static_web(json!({"permissions": ["models"]}));
    assert!(rejected(&other_permission).contains("models"));
}

#[test]
fn server_only_fields_are_rejected_not_ignored() {
    for (field, value) in [
        ("secrets", json!({"k": {"input": {"title": {"en": "Key"}}}})),
        ("settings", json!("settings.json")),
        ("setup", json!({"en": "Do this"})),
        ("webui", json!({"service": "web", "port": 80})),
    ] {
        let doc = static_web(json!({ field: value }));
        assert!(rejected(&doc).contains(field), "{field}");
    }
    let mut doc = base("static_web");
    doc["services"] = json!({"web": {"image": "nginx"}});
    assert!(matches!(parse(&doc), Err(PackageError::Service { .. })));
    let doc = static_web(json!({"requires": ["models"]}));
    assert!(rejected(&doc).contains("web_storage"));
    assert!(parse(&static_web(json!({"requires": ["web_storage"]}))).is_ok());
}

#[test]
fn link_takes_one_https_address_and_nothing_else() {
    let parsed = manifest(&link("https://example.com/dashboard"));
    assert_eq!(parsed.runtime, Runtime::Link);
    assert_eq!(PluginRuntime::from(parsed.runtime), PluginRuntime::Link);
    assert_eq!(parsed.link.unwrap().url, "https://example.com/dashboard");

    for url in [
        "http://example.com",
        "https://user:pass@example.com",
        "javascript:alert(1)",
        "not a url",
    ] {
        assert!(rejected(&link(url)).contains("link.url"), "{url}");
    }
    assert!(rejected(&base("link")).contains("link"));
    let mut with_permission = link("https://example.com");
    with_permission["x-wonton"]["permissions"] = json!(["network.egress"]);
    assert!(rejected(&with_permission).contains("permissions"));
}

#[test]
fn form_fields_only_go_with_their_runtime() {
    let mut doc = base("compose");
    doc["services"] = json!({"web": {"image": "nginx"}});
    doc["x-wonton"]["static_web"] = json!({});
    assert!(rejected(&doc).contains("static_web"));
    doc["x-wonton"]
        .as_object_mut()
        .unwrap()
        .remove("static_web");
    doc["x-wonton"]["link"] = json!({"url": "https://example.com"});
    assert!(rejected(&doc).contains("link"));
}

#[test]
fn security_tier_is_computed_from_the_manifest() {
    assert_eq!(
        security_tier(&manifest(&base("static_web"))),
        SecurityTier::Minimal
    );
    assert_eq!(
        security_tier(&manifest(&link("https://example.com"))),
        SecurityTier::Minimal
    );
    let connects = static_web(json!({
        "permissions": ["network.egress"],
        "static_web": {"connect": ["https://api.example.com"]}
    }));
    assert_eq!(security_tier(&manifest(&connects)), SecurityTier::Standard);

    for (permissions, optional, tier) in [
        (json!(["models"]), json!({}), SecurityTier::Standard),
        (json!(["network.lan"]), json!({}), SecurityTier::High),
        (json!(["network.host"]), json!({}), SecurityTier::VeryHigh),
        (json!(["docker.socket"]), json!({}), SecurityTier::Highest),
        (
            json!([]),
            json!({"docker.socket": {"off_loss": {"zh": "管不了容器", "en": "No containers"}}}),
            SecurityTier::Highest,
        ),
    ] {
        let mut doc = base("compose");
        let mut service = json!({"image": "app"});
        if permissions == json!(["network.lan"]) {
            doc["x-wonton"]["lan"] = json!({"ports": [{"service": "web", "port": 8080}]});
        }
        if permissions == json!(["network.host"]) {
            service["network_mode"] = json!("host");
        }
        if permissions == json!(["docker.socket"]) || optional != json!({}) {
            service["volumes"] = json!(["/var/run/docker.sock:/var/run/docker.sock"]);
        }
        doc["services"] = json!({ "web": service });
        doc["x-wonton"]["permissions"] = permissions;
        doc["x-wonton"]["optional_permissions"] = optional;
        assert_eq!(security_tier(&manifest(&doc)), tier, "{doc}");
        // 商店里要不要标红跟档位走：High 及以上（含比这个 App 新的档位）才标。
        assert_eq!(prominent(tier), tier >= SecurityTier::High, "{tier:?}");
        assert!(prominent(SecurityTier::Other), "未知的档位按最高档画");
    }
}

#[test]
fn hosted_capabilities_need_a_mount_and_others_refuse_one() {
    let provider = |name: &str, mount: Option<&str>| {
        let mut doc = base("compose");
        doc["x-wonton"]["kind"] = json!("system");
        doc["services"] = json!({"web": {"image": "static-web-server"}});
        let mut entry = json!({"service": "web", "port": 8080});
        if let Some(mount) = mount {
            entry["mount"] = json!(mount);
        }
        doc["x-wonton"]["provides"] = json!({ name: entry });
        doc
    };
    for name in ["static_web", "web_storage"] {
        let parsed = manifest(&provider(name, Some("/srv/wonton")));
        let provides = parsed.provides.unwrap();
        assert_eq!(
            provides.capabilities[name].mount.as_deref(),
            Some("/srv/wonton")
        );
        assert!(rejected(&provider(name, None)).contains("mount"), "{name}");
        for mount in ["srv", "/", "/proc/x", "/srv/../etc"] {
            assert!(
                rejected(&provider(name, Some(mount))).contains("mount"),
                "{name} {mount}"
            );
        }
    }
    assert!(rejected(&provider("torrent_client", Some("/srv"))).contains("mount"));
}

#[test]
fn provider_mount_is_omitted_when_absent_and_kept_when_present() {
    use super::super::provides::ProviderService;

    let without: ProviderService =
        serde_json::from_value(json!({"service": "web", "port": 8080})).unwrap();
    assert_eq!(without.mount, None);
    assert_eq!(
        serde_json::to_value(&without).unwrap(),
        json!({"service": "web", "port": 8080})
    );
    let with: ProviderService =
        serde_json::from_value(json!({"service": "web", "port": 8080, "mount": "/srv/wonton"}))
            .unwrap();
    assert_eq!(with.mount.as_deref(), Some("/srv/wonton"));
    assert_eq!(
        serde_json::to_value(&with).unwrap(),
        json!({"service": "web", "port": 8080, "mount": "/srv/wonton"})
    );
    let hosted = |name| {
        super::super::capabilities::lookup(name)
            .and_then(|c| c.hosted)
            .unwrap()
    };
    assert!(hosted("static_web").mount.is_some());
    assert!(hosted("web_storage").mount.is_some());
}

#[test]
fn new_fields_and_runtimes_need_spec_0_12() {
    let mut old = static_web(json!({"static_web": {"root": "web"}}));
    old["x-wonton"]["spec"] = json!("0.11");
    let found = spec_floor::problems(&old);
    assert!(
        found.iter().any(|p| p.contains("runtime: static_web")),
        "{found:?}"
    );
    assert!(
        found.iter().any(|p| p.contains("x-wonton.static_web")),
        "{found:?}"
    );
    let mut old_link = link("https://example.com");
    old_link["x-wonton"]["spec"] = json!("0.11");
    assert_eq!(spec_floor::problems(&old_link).len(), 2);
    assert!(spec_floor::problems(&static_web(json!({}))).is_empty());
}

#[tokio::test]
async fn hosted_forms_are_refused_at_install_until_hosting_exists() {
    let (_docker, engine) = super::super::fake_docker::FakeDocker::start().await;
    let dir = tempfile::tempdir().unwrap();
    let manager = super::manager::manager_with(engine, &dir);
    for doc in [base("static_web"), link("https://example.com")] {
        let package = parse(&doc).unwrap();
        let error = manager
            .install_package(package, &[], super::langs())
            .await
            .unwrap_err();
        assert_eq!(error.kind(), "unsupported");
    }
    assert!(manager.list().is_empty());
}
