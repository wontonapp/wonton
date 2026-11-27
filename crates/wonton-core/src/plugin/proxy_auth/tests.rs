use std::collections::BTreeMap;

use hyper::header::HeaderName;

use super::*;
use crate::owner::Owner;
use crate::plugin::PluginPackage;
use crate::plugin::languages::Languages;
use crate::plugin::manifest::Manifest;
use crate::plugin::secrets::Secrets;

fn manifest(extra: &str) -> Manifest {
    let source = format!(
        "services:\n  web:\n    image: example/app\nx-wonton:\n  spec: '0.9'\n  \
             id: app.test\n  version: 1.0.0\n  kind: app\n  title: {{en: Test}}\n  \
             description: {{en: Test}}\n  permissions: []\n{extra}"
    );
    PluginPackage::parse(&source, &Languages::default())
        .unwrap()
        .manifest
}

#[test]
fn lan_templates_reject_roles_at_validation_and_rendering() {
    let manifest = manifest("");
    let owner = Owner {
        username: "owner".into(),
        display_name: "Owner".into(),
        email: None,
    };
    let langs = Languages::default();
    for template in ["{role}", "prefix-{role}-{user}"] {
        assert!(check_template(&manifest, template, &langs).is_err());
        assert!(fill_template(template, &owner, &BTreeMap::new(), &langs).is_err());
    }
    assert!(check_template(&manifest, "{user}", &langs).is_ok());
    assert_eq!(
        fill_template("{user}", &owner, &BTreeMap::new(), &langs).unwrap(),
        "owner"
    );
}

#[test]
fn mcp_does_not_gain_a_request_role_context() {
    let mut manifest = manifest(
        "  mcp: {service: web, port: 80, transport: streamable-http, headers: {X-Key: value}}",
    );
    manifest
        .mcp
        .as_mut()
        .unwrap()
        .headers
        .insert("X-Role".into(), "{role}".into());
    let langs = Languages::default();
    assert!(validate_mcp_headers(&manifest, &langs).is_err());
    assert!(resolve_mcp_headers(&manifest, &Secrets::new(), &langs).is_err());
}

/// 用户填写的密钥可以出现在 `mcp.headers` 里：没填时头先空着，填了照常注入。
#[test]
fn mcp_headers_take_a_user_filled_key_and_tolerate_it_being_missing() {
    let manifest = manifest(
        "  mcp: {service: web, port: 80, transport: streamable-http, headers: {Authorization: 'Bearer {secret:token}'}}\n  \
             secrets: {token: {input: {title: {en: Token}}}}",
    );
    let langs = Languages::default();
    assert!(validate_mcp_headers(&manifest, &langs).is_ok());
    let pending = resolve_mcp_headers_or_pending(&manifest, &Secrets::new(), &langs).unwrap();
    assert!(pending.custom_headers().is_empty(), "没填时先空着");
    let filled: Secrets = [("token".to_owned(), b"abc".to_vec())].into();
    let resolved = resolve_mcp_headers_or_pending(&manifest, &filled, &langs).unwrap();
    let value = resolved
        .custom_headers()
        .get(&HeaderName::from_static("authorization"))
        .cloned()
        .expect("填了就有头");
    assert_eq!(value, "Bearer abc");
}

/// 同一份头里既有没填的用户输入、又有没收到的 `collect` 项：`collect` 是部署问题，不能说“等用户填”
/// 整条吞掉，要报错并点名那个 `collect` 项。
#[test]
fn a_pending_input_does_not_hide_a_missing_collected_key() {
    let manifest = manifest(
        "  mcp: {service: web, port: 80, transport: streamable-http, headers: {A-Token: '{secret:pat}', Authorization: 'Bearer {secret:token}'}}\n  \
             secrets: {token: {input: {title: {en: Token}}}, pat: {collect: {service: web, path: /run/token}}}",
    );
    let langs = Languages::default();
    assert!(validate_mcp_headers(&manifest, &langs).is_ok());
    // 两个都没值：旧实现把“有没填的 input”当理由整条放行空头；新实现要点名那个 collect 项。
    let error = resolve_mcp_headers_or_pending(&manifest, &Secrets::new(), &langs).unwrap_err();
    assert!(error.contains("pat"), "{error}");
    // 只收下 collect 项、input 仍空着：仍算等用户填，不报错。
    let only_pat: Secrets = [("pat".to_owned(), b"tok".to_vec())].into();
    let pending = resolve_mcp_headers_or_pending(&manifest, &only_pat, &langs).unwrap();
    assert!(pending.custom_headers().is_empty(), "等用户填时先空着");
    // 都齐了照常注入两个头。
    let both: Secrets = [
        ("token".to_owned(), b"abc".to_vec()),
        ("pat".to_owned(), b"tok".to_vec()),
    ]
    .into();
    let resolved = resolve_mcp_headers_or_pending(&manifest, &both, &langs).unwrap();
    assert_eq!(resolved.custom_headers().len(), 2);
}

/// 契约：`mcp.headers` 引用没声明的密钥是清单错误。清单解析时就校验头，所以在解析完的清单上改出来再验。
#[test]
fn an_mcp_header_naming_an_undeclared_key_is_invalid() {
    let mut manifest = manifest(
        "  mcp: {service: web, port: 80, transport: streamable-http, headers: {X-Key: value}}",
    );
    manifest
        .mcp
        .as_mut()
        .unwrap()
        .headers
        .insert("X-Key".into(), "{secret:nope}".into());
    let err = validate_mcp_headers(&manifest, &Languages::default()).unwrap_err();
    assert!(err.to_string().contains("nope"), "{err}");
}
