use serde_json::{Map, Value, json};

use super::*;
use crate::plugin::languages::Languages;
use crate::plugin::manifest::{Manifest, Permission};

fn base() -> Map<String, Value> {
    serde_json::from_value(json!({
        "spec": "0.14",
        "id": "com.example.clients",
        "version": "1.0.0",
        "kind": "app",
        "title": {"en": "Example"},
        "description": {"en": "Example"},
        "permissions": ["network.clients"],
        "webui": {"service": "web", "port": 8080}
    }))
    .unwrap()
}

fn with(fields: Value) -> Manifest {
    let mut object = base();
    object.extend(fields.as_object().unwrap().clone());
    serde_json::from_value(Value::Object(object)).unwrap()
}

fn langs() -> Languages {
    Languages::from_header(Some("zh-Hans"))
}

fn errors(manifest: &Manifest) -> String {
    manifest
        .validate(&langs())
        .expect_err("manifest should be invalid")
        .to_string()
}

const FULL: &str = r#"{
    "http": {
        "service": "forgejo",
        "port": 3000,
        "single_host": true,
        "auth": {"credentials": {
            "issue": ["sh", "/wonton/token.sh", "issue", "{name}"],
            "revoke": ["sh", "/wonton/token.sh", "revoke", "{name}"],
            "ends_sessions": true,
            "generates": false
        }},
        "public": {"allow": ["/webhook", "/form"]},
        "auth_failures": [{"path": "/user/login", "status": [401, 403]}]
    },
    "tcp": [{"service": "forgejo", "port": 22, "name": "ssh"}]
}"#;

#[test]
fn clients_manifest_parses() {
    let manifest: Manifest = with(json!({"clients": serde_json::from_str::<Value>(FULL).unwrap()}));
    let clients = manifest.clients.as_ref().unwrap();
    let http = clients.http.as_ref().unwrap();
    assert_eq!(http.service, "forgejo");
    assert_eq!(http.port, 3000);
    assert!(http.single_host);
    let credentials = http.auth.credentials.as_ref().unwrap();
    assert!(credentials.ends_sessions);
    assert!(credentials.per_client);
    assert!(!credentials.generates);
    assert_eq!(credentials.rotate, None);
    assert_eq!(credentials.service, None);
    assert_eq!(credentials.username, None);
    assert_eq!(
        http.public.as_ref().unwrap().allow.as_deref(),
        Some(&["/webhook".to_owned(), "/form".to_owned()][..])
    );
    let failures = http.auth_failures.as_ref().unwrap();
    assert_eq!(failures[0].path, "/user/login");
    assert_eq!(failures[0].status, vec![401, 403]);
    let tcp = clients.tcp.as_ref().unwrap();
    assert_eq!(tcp[0].name, "ssh");
    assert_eq!(tcp[0].port, 22);
    manifest.validate(&langs()).unwrap();
    let encoded = serde_json::to_value(&manifest).unwrap();
    assert_eq!(encoded["clients"]["http"]["service"], "forgejo");
}

#[test]
fn optional_clients_fields_stay_out_when_not_written() {
    let manifest = with(json!({"clients": {"http": {
        "service": "web",
        "port": 3000,
        "auth": {"app": {"setup_done": ["sh", "/wonton/setup-done.sh"]}}
    }}}));
    manifest.validate(&langs()).unwrap();
    let http = manifest.clients.as_ref().unwrap().http.as_ref().unwrap();
    assert!(!http.single_host);
    assert!(http.public.is_none());
    assert!(http.auth_failures.is_none());
    let encoded = serde_json::to_value(&manifest).unwrap();
    assert!(encoded["clients"].get("tcp").is_none());
    assert!(encoded["clients"]["http"].get("single_host").is_none());
}

#[test]
fn clients_requires_permission_and_vice_versa() {
    let without_permission: Manifest = serde_json::from_value(json!({
        "spec": "0.14",
        "id": "com.example.clients",
        "version": "1.0.0",
        "kind": "app",
        "title": {"en": "Example"},
        "description": {"en": "Example"},
        "permissions": [],
        "clients": {"tcp": [{"service": "web", "port": 22, "name": "ssh"}]}
    }))
    .unwrap();
    let message = errors(&without_permission);
    assert!(message.contains("network.clients"), "{message}");

    let without_clients: Manifest = serde_json::from_value(json!({
        "spec": "0.14",
        "id": "com.example.clients",
        "version": "1.0.0",
        "kind": "app",
        "title": {"en": "Example"},
        "description": {"en": "Example"},
        "permissions": ["network.clients"]
    }))
    .unwrap();
    let message = errors(&without_clients);
    assert!(message.contains("clients"), "{message}");
}

#[test]
fn clients_needs_http_or_tcp() {
    let message = errors(&with(json!({"clients": {}})));
    assert!(message.contains("http 或 tcp"), "{message}");
    let message = errors(&with(json!({"clients": {"tcp": []}})));
    assert!(message.contains("http 或 tcp"), "{message}");
}

#[test]
fn public_allow_and_deny_exclusive() {
    let manifest = with(json!({"clients": {"http": {
        "service": "web",
        "port": 3000,
        "auth": {"app": {"setup_done": ["sh", "/wonton/setup-done.sh"]}},
        "public": {"allow": ["/a"], "deny": ["/b"]}
    }}}));
    let message = errors(&manifest);
    assert!(message.contains("allow 和 deny"), "{message}");
}

#[test]
fn public_paths_reject_unsafe_chars() {
    for bad in [
        "/a%2f", "/a/../b", "/a;b", "/a b", "a", "/a\\b", "/a?x=1", "/a#x", "/a/./b",
    ] {
        let manifest = with(json!({"clients": {"http": {
            "service": "web",
            "port": 3000,
            "auth": {"app": {"setup_done": ["sh", "/wonton/setup-done.sh"]}},
            "public": {"allow": [bad]}
        }}}));
        let message = errors(&manifest);
        assert!(message.contains(bad), "{bad}: {message}");
    }
    let manifest = with(json!({"clients": {"http": {
        "service": "web",
        "port": 3000,
        "auth": {"app": {"setup_done": ["sh", "/wonton/setup-done.sh"]}},
        "public": {"allow": ["/webhook-test", "/a/b.c~d"]}
    }}}));
    manifest.validate(&langs()).unwrap();
}

#[test]
fn auth_failures_paths_and_statuses() {
    let manifest = with(json!({"clients": {"http": {
        "service": "web",
        "port": 3000,
        "auth": {"app": {"setup_done": ["sh", "/wonton/setup-done.sh"]}},
        "auth_failures": [{"path": "/a%2f", "status": [200, 401, 700]}]
    }}}));
    let message = errors(&manifest);
    assert!(message.contains("/a%2f"), "{message}");
    assert!(message.contains("200"), "{message}");
    assert!(message.contains("700"), "{message}");
    let manifest = with(json!({"clients": {"http": {
        "service": "web",
        "port": 3000,
        "auth": {"app": {"setup_done": ["sh", "/wonton/setup-done.sh"]}},
        "auth_failures": [{"path": "/api/auth/login", "status": [401]}]
    }}}));
    manifest.validate(&langs()).unwrap();
}

fn credentials(extra: Value) -> Manifest {
    let mut credentials = json!({
        "issue": ["sh", "/wonton/token.sh"],
        "ends_sessions": true
    });
    credentials
        .as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    with(json!({"clients": {"http": {
        "service": "web",
        "port": 3000,
        "auth": {"credentials": credentials}
    }}}))
}

#[test]
fn credentials_shape_rules() {
    let message = errors(&credentials(json!({})));
    assert!(message.contains("revoke"), "{message}");

    credentials(json!({"revoke": ["sh", "/wonton/token.sh", "revoke"]}))
        .validate(&langs())
        .unwrap();

    let message = errors(&credentials(json!({"per_client": false, "revoke": []})));
    assert!(message.contains("rotate"), "{message}");

    credentials(json!({"per_client": false, "rotate": ["sh", "/wonton/token.sh", "rotate"]}))
        .validate(&langs())
        .unwrap();

    let message = errors(&credentials(json!({
        "revoke": ["sh", "/wonton/token.sh", "revoke"],
        "generates": true
    })));
    assert!(message.contains("verify"), "{message}");

    credentials(json!({
        "revoke": ["sh", "/wonton/token.sh", "revoke"],
        "generates": true,
        "verify": ["sh", "/wonton/token.sh", "check"]
    }))
    .validate(&langs())
    .unwrap();
}

#[test]
fn credentials_needs_issue() {
    let manifest = with(json!({"clients": {"http": {
        "service": "web",
        "port": 3000,
        "auth": {"credentials": {
            "issue": [],
            "revoke": ["sh", "/wonton/token.sh", "revoke"],
            "ends_sessions": false
        }}
    }}}));
    let message = errors(&manifest);
    assert!(message.contains("issue"), "{message}");
}

#[test]
fn app_requires_setup_done() {
    let manifest = with(json!({"clients": {"http": {
        "service": "web",
        "port": 3000,
        "auth": {"app": {"setup_done": []}}
    }}}));
    let message = errors(&manifest);
    assert!(message.contains("setup_done"), "{message}");
}

#[test]
fn gateway_ignores_auth_failures_rejected() {
    let manifest = with(json!({"clients": {"http": {
        "service": "web",
        "port": 3000,
        "auth": {"gateway": {"headers": {"X-Remote-User": "{user}"}}},
        "auth_failures": [{"path": "/user/login", "status": [401]}]
    }}}));
    let message = errors(&manifest);
    assert!(message.contains("auth_failures"), "{message}");
    let manifest = with(json!({"clients": {"http": {
        "service": "web",
        "port": 3000,
        "auth": {"gateway": {"headers": {"X-Remote-User": "{user}"}}}
    }}}));
    manifest.validate(&langs()).unwrap();
}

#[test]
fn gateway_headers_use_the_proxy_rules() {
    for (name, value) in [("Host", "example"), ("X-Forwarded-For", "example")] {
        let mut headers = serde_json::Map::new();
        headers.insert(name.to_owned(), json!(value));
        let manifest = with(json!({"clients": {"http": {
            "service": "web",
            "port": 3000,
            "auth": {"gateway": {"headers": headers}}
        }}}));
        let message = errors(&manifest);
        assert!(message.contains(name), "{name}: {message}");
    }
}

#[test]
fn exactly_one_auth_mode() {
    let none = with(json!({"clients": {"http": {"service": "web", "port": 3000, "auth": {}}}}));
    let message = errors(&none);
    assert!(message.contains("恰好"), "{message}");
    let two = with(json!({"clients": {"http": {
        "service": "web",
        "port": 3000,
        "auth": {
            "app": {"setup_done": ["sh", "/wonton/setup-done.sh"]},
            "gateway": {"headers": {}}
        }
    }}}));
    let message = errors(&two);
    assert!(message.contains("恰好"), "{message}");
}

#[test]
fn http_port_cannot_be_mcp_port() {
    let manifest: Manifest = serde_json::from_value(json!({
        "spec": "0.14",
        "id": "com.example.clients",
        "version": "1.0.0",
        "kind": "app",
        "title": {"en": "Example"},
        "description": {"en": "Example"},
        "permissions": ["network.clients"],
        "mcp": {"service": "web", "port": 3000, "transport": "streamable-http"},
        "clients": {"http": {
            "service": "web",
            "port": 3000,
            "auth": {"app": {"setup_done": ["sh", "/wonton/setup-done.sh"]}}
        }}
    }))
    .unwrap();
    let message = errors(&manifest);
    assert!(message.contains("mcp"), "{message}");
}

#[test]
fn http_port_may_be_the_webui_port() {
    let manifest = with(json!({"clients": {"http": {
        "service": "web",
        "port": 8080,
        "auth": {"app": {"setup_done": ["sh", "/wonton/setup-done.sh"]}}
    }}}));
    manifest.validate(&langs()).unwrap();
}

#[test]
fn tcp_ports_cannot_be_webui_or_mcp() {
    let webui =
        with(json!({"clients": {"tcp": [{"service": "web", "port": 8080, "name": "ssh"}]}}));
    let message = errors(&webui);
    assert!(message.contains("8080"), "{message}");

    let mcp: Manifest = serde_json::from_value(json!({
        "spec": "0.14",
        "id": "com.example.clients",
        "version": "1.0.0",
        "kind": "app",
        "title": {"en": "Example"},
        "description": {"en": "Example"},
        "permissions": ["network.clients"],
        "mcp": {"service": "web", "port": 9000, "transport": "streamable-http"},
        "clients": {"tcp": [{"service": "web", "port": 9000, "name": "ssh"}]}
    }))
    .unwrap();
    let message = errors(&mcp);
    assert!(message.contains("mcp"), "{message}");
}

#[test]
fn tcp_names_unique_and_well_formed() {
    let manifest = with(json!({"clients": {"tcp": [
        {"service": "web", "port": 22, "name": "ssh"},
        {"service": "web", "port": 23, "name": "ssh"}
    ]}}));
    let message = errors(&manifest);
    assert!(message.contains("重复"), "{message}");
    let manifest =
        with(json!({"clients": {"tcp": [{"service": "web", "port": 22, "name": "Git_SSH"}]}}));
    let message = errors(&manifest);
    assert!(message.contains("Git_SSH"), "{message}");
}

#[test]
fn external_url_only_with_clients_or_guests() {
    let langs = langs();
    let plain = with(json!({"permissions": []}));
    let message = validate_external_url(&plain, true, &langs)
        .unwrap_err()
        .to_string();
    assert!(message.contains("WONTON_EXTERNAL_URL"), "{message}");
    validate_external_url(&plain, false, &langs).unwrap();

    let http = with(json!({"clients": {"http": {
        "service": "web",
        "port": 3000,
        "auth": {"app": {"setup_done": ["sh", "/wonton/setup-done.sh"]}}
    }}}));
    validate_external_url(&http, true, &langs).unwrap();

    let mut guests = plain.clone();
    guests.webui.as_mut().unwrap().guests = true;
    validate_external_url(&guests, true, &langs).unwrap();
}

#[test]
fn guests_requires_webui() {
    // guests 只能写在 webui 里，写在清单顶层认不出来。
    let error = serde_json::from_value::<Manifest>(json!({
        "spec": "0.14",
        "id": "com.example.clients",
        "version": "1.0.0",
        "kind": "app",
        "title": {"en": "Example"},
        "description": {"en": "Example"},
        "permissions": ["network.clients"],
        "guests": true
    }))
    .unwrap_err()
    .to_string();
    assert!(error.contains("guests"), "{error}");

    let manifest = with(json!({"permissions": ["network.clients"], "webui": {
        "service": "web",
        "port": 8080,
        "guests": true
    }}));
    assert!(manifest.webui.as_ref().unwrap().guests);
    manifest.validate(&langs()).unwrap();
}

#[test]
fn guests_needs_permission() {
    let manifest = with(json!({"permissions": [], "webui": {
        "service": "web",
        "port": 8080,
        "guests": true
    }}));
    let message = errors(&manifest);
    assert!(message.contains("network.clients"), "{message}");
}

#[test]
fn unknown_clients_field_rejected() {
    let error = serde_json::from_value::<Manifest>(json!({
        "spec": "0.14",
        "id": "com.example.clients",
        "version": "1.0.0",
        "kind": "app",
        "title": {"en": "Example"},
        "description": {"en": "Example"},
        "permissions": ["network.clients"],
        "clients": {"http": {
            "service": "web",
            "port": 3000,
            "bogus": 1,
            "auth": {"app": {"setup_done": ["sh", "/wonton/setup-done.sh"]}}
        }}
    }))
    .unwrap_err()
    .to_string();
    assert!(error.contains("bogus"), "{error}");
}

#[test]
fn all_problems_reported_at_once() {
    let manifest = with(json!({"clients": {"http": {
        "service": "web",
        "port": 0,
        "auth": {"app": {"setup_done": []}},
        "public": {"allow": ["a%2f"], "deny": ["/b"]}
    }}}));
    let message = errors(&manifest);
    assert!(message.lines().count() >= 4, "{message}");
    for part in ["端口", "setup_done", "allow 和 deny", "a%2f"] {
        assert!(message.contains(part), "{part}: {message}");
    }
}

#[test]
fn network_clients_is_not_high_privilege() {
    assert!(!Permission::NetworkClients.is_high());
    assert_eq!(
        crate::plugin::views_permissions::permission_risk(&Permission::NetworkClients),
        None
    );
    assert!(Permission::NetworkClients.to_string() == "network.clients");
    assert_eq!(
        "network.clients".parse::<Permission>(),
        Ok(Permission::NetworkClients)
    );
}
