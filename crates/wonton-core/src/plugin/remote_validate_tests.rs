use serde_json::{Value, json};

use super::*;
use crate::plugin::install_error::InstallError;
use crate::plugin::package::{PackageError, PluginPackage};
use crate::plugin::proxy_auth::{McpHeaders, render_input_header, render_input_url};
use crate::plugin::secrets::Secrets;

fn document() -> Value {
    json!({"x-wonton": {
        "spec": "0.15", "id": "com.example.remote", "version": "1.0.0", "kind": "app",
        "runtime": "remote_mcp", "title": {"en": "Remote tools"},
        "description": {"en": "Remote tools"}, "permissions": [],
        "remote_mcp": {"url": "https://mcp.example.com/mcp"}
    }})
}

fn input_secret(doc: &mut Value) {
    doc["x-wonton"]["secrets"] = json!({"token": {"input": {"title": {"en": "Token"}}}});
}

fn parse(doc: &Value) -> Result<PluginPackage, PackageError> {
    PluginPackage::parse(&doc.to_string(), &Languages::default())
}

fn rejected(doc: &Value) -> String {
    let error = parse(doc).unwrap_err();
    assert!(
        matches!(&error, PackageError::Manifest(ManifestError::Invalid(_))),
        "{error}"
    );
    let error = InstallError::from(error);
    assert_eq!(error.kind(), "invalid_input");
    error.to_string()
}

fn container_fields() -> Vec<(&'static str, Value)> {
    vec![
        ("services", json!({})),
        ("webui", Value::Null),
        ("tabs", json!([])),
        ("mcp", Value::Null),
        ("bot", Value::Null),
        ("bots", Value::Null),
        ("agent", Value::Null),
        ("provides", Value::Null),
        ("requires", json!([])),
        ("wants", json!([])),
        ("gpu", Value::Null),
        ("lan", Value::Null),
        ("tunnel", Value::Null),
        ("setup", Value::Null),
        ("settings", Value::Null),
        ("requirements", Value::Null),
        ("optional_permissions", json!({})),
        ("permissions", json!(["network.egress"])),
        ("devices", Value::Null),
        ("clients", Value::Null),
        ("matrix_widget", Value::Null),
        ("static_web", Value::Null),
        ("link", Value::Null),
        ("install_timeout_secs", Value::Null),
    ]
}

fn set_field(doc: &mut Value, name: &str, value: Value) {
    if name == "services" {
        doc[name] = value;
    } else {
        doc["x-wonton"][name] = value;
    }
}

#[test]
fn remote_mcp_rejects_container_fields() {
    for (name, value) in container_fields() {
        let mut doc = document();
        set_field(&mut doc, name, value);
        let error = rejected(&doc);
        assert!(error.contains(name), "missing {name}: {error}");
    }
    let mut all = document();
    for (name, value) in container_fields() {
        set_field(&mut all, name, value);
    }
    let error = rejected(&all);
    for (name, _) in container_fields() {
        assert!(error.contains(name), "missing {name}: {error}");
    }
}

#[test]
fn remote_mcp_manifest_validation_precedes_container_validation() {
    let mut doc = document();
    doc["x-wonton"]["tabs"] = json!([]);
    doc["x-wonton"]["permissions"] = json!(["network.egress"]);
    let manifest: Manifest = serde_json::from_value(doc["x-wonton"].take()).unwrap();
    let error = manifest.validate(&Languages::default()).unwrap_err();
    assert!(matches!(&error, ManifestError::Invalid(_)));
    let error = error.to_string();
    for field in ["tabs", "permissions"] {
        assert!(error.contains(field), "{error}");
    }
}

#[test]
fn remote_mcp_url_rules() {
    for url in [
        "http://mcp.example.com/",
        "https://1.2.3.4/",
        "https://[::1]/",
        "https://[::ffff:127.0.0.1]/",
        "https://127.1/",
        "https://2130706433/",
        "https://0x7f000001/",
        "https://localhost/",
        "https://a.localhost/",
        "https://LOCALHOST./",
        "https://intranet/",
        "https://intranet./",
        "https://u:p@example.com/",
        "https://@example.com/",
        "https://mcp.example.com/#",
        "https://mcp.example.com/#fragment",
        "https://mcp.example.com/white space",
        "https://mcp.example.com/\n",
        "https://mcp.example.com:bad/",
        "https:///mcp.example.com/",
        "https://mcp.example.com\\other/path",
    ] {
        let mut doc = document();
        doc["x-wonton"]["remote_mcp"]["url"] = json!(url);
        rejected(&doc);
    }
    for url in [
        "https://mcp.example.com/mcp",
        "https://mcp.example.com:8443/mcp?query=yes",
    ] {
        let mut doc = document();
        doc["x-wonton"]["remote_mcp"]["url"] = json!(url);
        parse(&doc).unwrap();
    }
}

#[test]
fn remote_mcp_url_templates_only_reference_input_secrets_in_path_and_query() {
    for url in [
        "{secret:token}://mcp.example.com/mcp",
        "https://{secret:token}.example.com/mcp",
        "https://mcp.example.com:{secret:token}/mcp",
        "https://{secret:token}@mcp.example.com/mcp",
        "https://mcp.example.com/{secret:unknown}",
        "https://mcp.example.com/{user}",
        "https://mcp.example.com/{name}",
        "https://mcp.example.com/{email}",
        "https://mcp.example.com/{role}",
        "https://mcp.example.com/{secret:}",
        "https://mcp.example.com/{secret:token",
        "https://mcp.example.com/token}",
        "https://mcp.example.com/{secret:{secret:token}}",
        "https://mcp.example.com/mcp#{secret:token}",
    ] {
        let mut doc = document();
        input_secret(&mut doc);
        doc["x-wonton"]["remote_mcp"]["url"] = json!(url);
        rejected(&doc);
    }
    for url in [
        "https://mcp.example.com/{secret:token}/mcp",
        "https://mcp.example.com/mcp?key={secret:token}",
        "https://mcp.example.com?{secret:token}=value",
        "https://mcp.example.com:443/{secret:token}?key={secret:token}",
    ] {
        let mut doc = document();
        input_secret(&mut doc);
        doc["x-wonton"]["remote_mcp"]["url"] = json!(url);
        parse(&doc).unwrap();
    }
    let mut generated = document();
    generated["x-wonton"]["secrets"] = json!({"token": {"generate": {"bytes": 32}}});
    generated["x-wonton"]["remote_mcp"]["url"] = json!("https://mcp.example.com/{secret:token}");
    rejected(&generated);
}

#[test]
fn remote_mcp_auth_shapes() {
    let mut none_headers = document();
    none_headers["x-wonton"]["remote_mcp"]["headers"] = json!({"Authorization": "fixed"});
    rejected(&none_headers);
    let mut no_headers = document();
    no_headers["x-wonton"]["remote_mcp"]["auth"] = json!("header");
    rejected(&no_headers);
    for secrets in [
        json!({"token": {"generate": {"bytes": 32}}}),
        json!({"token": {"collect": {"service": "server", "path": "/token"}}}),
        json!({}),
    ] {
        let mut doc = document();
        doc["x-wonton"]["secrets"] = secrets;
        doc["x-wonton"]["remote_mcp"]["auth"] = json!("header");
        doc["x-wonton"]["remote_mcp"]["headers"] =
            json!({"Authorization": "Bearer {secret:token}"});
        rejected(&doc);
    }
    let mut oauth = document();
    oauth["x-wonton"]["remote_mcp"]["auth"] = json!("oauth");
    parse(&oauth).unwrap();
    input_secret(&mut oauth);
    rejected(&oauth);
    oauth["x-wonton"].as_object_mut().unwrap().remove("secrets");
    oauth["x-wonton"]["remote_mcp"]["headers"] = json!({"Authorization": "fixed"});
    rejected(&oauth);
    let mut header = document();
    input_secret(&mut header);
    header["x-wonton"]["remote_mcp"]["auth"] = json!("header");
    header["x-wonton"]["remote_mcp"]["headers"] = json!({"Authorization": "Bearer {secret:token}"});
    parse(&header).unwrap();
}

#[test]
fn remote_mcp_forbids_transport_and_core_headers() {
    for name in [
        "Host",
        "Cookie",
        "Forwarded",
        "X-Real-IP",
        "X-Forwarded-For",
        "Connection",
        "Keep-Alive",
        "Proxy-Authenticate",
        "Proxy-Authorization",
        "TE",
        "Trailer",
        "Transfer-Encoding",
        "Upgrade",
        "Mcp-Session-Id",
        "Mcp-Protocol-Version",
        "X-Wonton-Plugin",
        "x-wonton-anything",
        "Content-Length",
        "Last-Event-Id",
    ] {
        let mut doc = document();
        doc["x-wonton"]["remote_mcp"]["auth"] = json!("header");
        doc["x-wonton"]["remote_mcp"]["headers"] = json!({name: "fixed"});
        let error = rejected(&doc);
        assert!(error.contains(name), "{error}");
    }
    for value in [
        "{user}",
        "{name}",
        "{email}",
        "{role}",
        "{secret:}",
        "{secret:missing}",
        "{secret:token",
        "value\r\ninjected: yes",
    ] {
        let mut doc = document();
        input_secret(&mut doc);
        doc["x-wonton"]["remote_mcp"]["auth"] = json!("header");
        doc["x-wonton"]["remote_mcp"]["headers"] = json!({"Authorization": value});
        rejected(&doc);
    }
}

#[test]
fn remote_mcp_aggregates_fields_url_auth_and_secret_issues() {
    let mut doc = document();
    doc["services"] = json!({});
    doc["x-wonton"]["tabs"] = json!([]);
    doc["x-wonton"]["remote_mcp"] = json!({
        "url": "http://localhost/", "auth": "oauth", "headers": {"Host": "{secret:missing}"}
    });
    doc["x-wonton"]["secrets"] = json!({"generated": {"generate": {"bytes": 32}}});
    let error = rejected(&doc);
    for field in [
        "services",
        "tabs",
        "remote_mcp.url",
        "remote_mcp.headers",
        "Host",
        "missing",
        "generated",
        "secrets",
    ] {
        assert!(error.contains(field), "missing {field}: {error}");
    }
}

#[test]
fn remote_mcp_renders_url_components_and_header_values_without_leaking_errors() {
    let mut doc = document();
    input_secret(&mut doc);
    let manifest = parse(&doc).unwrap().manifest;
    let langs = Languages::default();
    let mut secrets = Secrets::from([("token".to_owned(), b"private/&?=#+ %\r\n".to_vec())]);
    let url = render_input_url(
        &manifest,
        &secrets,
        "https://mcp.example.com/{secret:token}?token={secret:token}&fixed=yes",
        &langs,
    )
    .unwrap();
    let parsed = Url::parse(&url).unwrap();
    assert_eq!(parsed.host_str(), Some("mcp.example.com"));
    assert_eq!(parsed.path(), "/private%2F%26%3F%3D%23%2B%20%25%0D%0A");
    assert_eq!(
        parsed.query_pairs().into_owned().collect::<Vec<_>>(),
        vec![
            ("token".to_owned(), "private/&?=#+ %\r\n".to_owned()),
            ("fixed".to_owned(), "yes".to_owned())
        ]
    );
    let error =
        render_input_header(&manifest, &secrets, "Bearer {secret:token}", &langs).unwrap_err();
    assert!(!error.contains("private"));
    secrets.insert("token".into(), b"private-valid-token".to_vec());
    assert_eq!(
        render_input_header(&manifest, &secrets, "Bearer {secret:token}", &langs).unwrap(),
        Some("Bearer private-valid-token".to_owned())
    );
    secrets.insert("token".into(), vec![0xff]);
    assert!(render_input_url(&manifest, &secrets, "{secret:token}", &langs).is_err());
    secrets.clear();
    assert!(render_input_header(&manifest, &secrets, "{secret:token}", &langs).is_err());
}

#[test]
fn remote_mcp_protocol_headers_are_sensitive_and_errors_name_only_the_header() {
    let headers = McpHeaders::from_sensitive_pairs([(
        "x-wonton-remote-url",
        "https://mcp.example.com/private-token".into(),
    )])
    .unwrap();
    assert!(
        headers
            .custom_headers()
            .values()
            .all(|value| value.is_sensitive())
    );
    assert!(!format!("{headers:?}").contains("private-token"));
    let error =
        McpHeaders::from_sensitive_pairs([("x-wonton-remote-url", "private-token\n".into())])
            .unwrap_err();
    assert_eq!(error, "x-wonton-remote-url");
}
