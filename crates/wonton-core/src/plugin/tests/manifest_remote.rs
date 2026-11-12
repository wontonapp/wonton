//! `runtime: remote_mcp` 的清单契约（ADR-0066 第 1、3 节）：地址、鉴权方式、请求头模板、工具范围和能力表中的
//! 托管形态。

use serde_json::{Value, json};

use super::super::capabilities;
use super::super::manifest::{Manifest, Runtime};
use super::super::package::{PackageError, PluginPackage};

fn base() -> Value {
    json!({"x-wonton": {
        "spec": "0.14", "id": "io.github.alice.mcp", "version": "1.0.0", "kind": "app",
        "runtime": "remote_mcp", "title": {"en": "Remote tools"}, "description": {"en": "Remote tools"},
        "permissions": [],
        "remote_mcp": {
            "url": "https://mcp.example.com/mcp",
            "auth": "oauth",
            "tools": ["search", "fetch"],
            "tool_idle_seconds": 600,
            "tool_total_seconds": 1800
        },
        "secrets": {"token": {"input": {"title": {"zh": "访问令牌", "en": "Access token"},
            "url": "https://example.com/keys"}}}
    }})
}

fn with(remote: Value) -> Value {
    let mut doc = base();
    doc["x-wonton"]["remote_mcp"] = remote;
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
fn remote_mcp_manifest_parses() {
    let parsed = manifest(&base());
    assert_eq!(parsed.runtime, Runtime::RemoteMcp);
    let remote = parsed.remote_mcp.unwrap();
    assert_eq!(remote.url, "https://mcp.example.com/mcp");
    assert_eq!(remote.auth, super::super::manifest_remote::RemoteAuth::Oauth);
    assert!(remote.headers.is_empty());
    assert_eq!(remote.tools.as_deref(), Some(&["search".into(), "fetch".into()][..]));
    assert_eq!(remote.tool_idle_seconds, Some(600));
    assert_eq!(remote.tool_total_seconds, Some(1800));
}

#[test]
fn remote_mcp_unknown_field_rejected() {
    let mut doc = base();
    doc["x-wonton"]["remote_mcp"]["foo"] = json!(true);
    let error = rejected(&doc);
    assert!(error.contains("foo"), "{error}");
}

/// `runtime: remote_mcp` 和 `remote_mcp` 字段必须两边都在；`url` 只收 `https`。
#[test]
fn remote_mcp_needs_its_runtime_and_an_https_url() {
    let mut compose = base();
    compose["x-wonton"]["runtime"] = json!("compose");
    compose["services"] = json!({"web": {"image": "nginx"}});
    assert!(rejected(&compose).contains("runtime: remote_mcp"));

    let mut missing = base();
    missing["x-wonton"].as_object_mut().unwrap().remove("remote_mcp");
    assert!(rejected(&missing).contains("remote_mcp.url"));

    assert!(rejected(&with(json!({"url": "http://mcp.example.com/mcp"}))).contains("https"));
    assert!(
        rejected(&with(json!({"url": "https://user:pw@mcp.example.com/mcp"}))).contains("凭据")
    );
    assert!(rejected(&with(json!({"url": "https://mcp.example.com/mcp#frag"}))).contains("#"));
}

/// `headers` 只和 `auth: header` 一起写，值模板只能引用 `input` 密钥，`X-Wonton-*` 和协议头由核心写。
#[test]
fn remote_headers_belong_to_header_auth_and_use_input_secrets() {
    let header = |headers: Value| json!({"url": "https://mcp.example.com/mcp",
        "auth": "header", "headers": headers});
    let parsed = manifest(&with(header(json!({"Authorization": "Bearer {secret:token}"}))));
    assert_eq!(
        parsed.remote_mcp.unwrap().headers["Authorization"],
        "Bearer {secret:token}"
    );

    assert!(rejected(&with(header(json!({})))).contains("headers"));
    assert!(
        rejected(&with(json!({"url": "https://mcp.example.com/mcp",
            "headers": {"Authorization": "Bearer {secret:token}"}})))
        .contains("auth: header")
    );

    let mut doc = base();
    doc["x-wonton"]["secrets"]["generated"] = json!({"generate": {"bytes": 32}});
    let error = rejected(&with(header(json!({"Authorization": "Bearer {secret:generated}"}))));
    assert!(error.contains("generated"), "{error}");

    for name in ["X-Wonton-Plugin", "Mcp-Session-Id", "Host"] {
        let error = rejected(&with(header(json!({name: "{secret:token}"}))));
        assert!(error.contains(name), "{name}: {error}");
    }
    let error = rejected(&with(header(json!({"X-Trace": "{user}"}))));
    assert!(error.contains("user"), "{error}");
}

/// 工具名和期限沿用 `mcp.*` 的规则，报错写 `remote_mcp` 的字段名。
#[test]
fn remote_tool_limits_follow_the_mcp_rules() {
    let error = rejected(&with(json!({"url": "https://mcp.example.com/mcp", "tools": []})));
    assert!(error.contains("remote_mcp.tools"), "{error}");
    let error = rejected(&with(json!({"url": "https://mcp.example.com/mcp",
        "tool_idle_seconds": 0})));
    assert!(error.contains("remote_mcp.tool_idle_seconds"), "{error}");
}

#[test]
fn remote_mcp_is_hosted_without_mount() {
    let capability = capabilities::lookup("remote_mcp").expect("在能力表里");
    let hosted = capability.hosted.expect("是托管形态");
    assert_eq!(hosted.runtime, Some(Runtime::RemoteMcp));
    assert_eq!(hosted.package_dir, None);
    assert_eq!(hosted.mount, None);
    assert!(!capability.high_privilege);
}
