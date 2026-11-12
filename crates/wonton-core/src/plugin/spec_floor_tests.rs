use serde_json::{Value, json};

use super::{TABLE, problems};
use crate::plugin::store_info::SPEC_VERSION;

fn document(spec: &str, manifest: Value, service: Value) -> Value {
    let mut x_wonton = json!({"spec": spec, "id": "app.test", "permissions": []});
    x_wonton
        .as_object_mut()
        .unwrap()
        .extend(manifest.as_object().unwrap().clone());
    json!({"services": {"web": service}, "x-wonton": x_wonton})
}

fn plain() -> Value {
    json!({"image": "nginx"})
}

#[test]
fn a_newer_field_under_an_older_spec_names_the_field_and_the_version() {
    let manifest =
        json!({"webui": {"service": "web", "port": 80, "oidc": {"post_logout_paths": ["/bye"]}}});
    let found = problems(&document("0.6", manifest.clone(), plain()));
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(
        found[0].contains("x-wonton.webui.oidc.post_logout_paths"),
        "{found:?}"
    );
    assert!(found[0].contains("\"0.7\""), "{found:?}");
    assert!(problems(&document("0.7", manifest, plain())).is_empty());
}

#[test]
fn setup_needs_spec_0_11() {
    let manifest = json!({"setup": {"zh": "把公钥加到宿主", "en": "Add the key"}});
    let found = problems(&document("0.10", manifest.clone(), plain()));
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(found[0].contains("x-wonton.setup"), "{found:?}");
    assert!(problems(&document("0.11", manifest, plain())).is_empty());
}

#[test]
fn compose_fields_and_permissions_count_too() {
    let manifest = json!({"permissions": ["plugins.health"]});
    let service = json!({"image": "nginx", "cap_drop": ["ALL"], "network_mode": "service:db"});
    let found = problems(&document("0.6", manifest, service));
    for place in [
        "permission plugins.health",
        "services.web.cap_drop",
        "services.web.network_mode",
    ] {
        assert!(
            found.iter().any(|p| p.contains(place)),
            "{place}: {found:?}"
        );
    }
    assert_eq!(found.len(), 3, "{found:?}");
}

/// 0.6 的核心要 `security.opt` 权限才收 `security_opt`；申请了权限的写法 0.6 就认得。
#[test]
fn no_new_privileges_needs_the_newer_spec_only_without_the_permission() {
    let service = json!({"image": "nginx", "security_opt": ["no-new-privileges:false"]});
    assert_eq!(
        problems(&document("0.6", json!({}), service.clone())).len(),
        1
    );
    let with_permission = json!({"permissions": ["security.opt"]});
    assert!(problems(&document("0.6", with_permission, service)).is_empty());
}

#[test]
fn a_manifest_that_uses_nothing_new_passes_under_any_spec() {
    let manifest = json!({"webui": {"service": "web", "port": 80}});
    assert!(problems(&document("0.5", manifest, plain())).is_empty());
}

/// 表里的版本不能比这份核心支持的新：那样的字段这份核心自己也读不懂。
#[test]
fn the_table_does_not_reach_past_the_core() {
    assert!(TABLE.iter().all(|(since, _)| *since <= SPEC_VERSION));
}

/// 按名字的表（`secrets`）里的新字段：`*` 展开成实际的键，一处一条；spec 写到那一版就不报。
#[test]
fn a_field_inside_a_name_keyed_table_names_the_actual_key() {
    let manifest = json!({"secrets": {
        "admin_token": {"generate": {}, "show_owner": {"label": {"en": "Admin token"}}},
        "db_password": {"generate": {}}
    }});
    let found = problems(&document("0.7", manifest.clone(), plain()));
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(
        found[0].contains("x-wonton.secrets.admin_token.show_owner"),
        "{found:?}"
    );
    assert!(found[0].contains("\"0.8\""), "{found:?}");
    assert!(problems(&document("0.8", manifest, plain())).is_empty());
}

/// `input` 也是 0.8 的写法，报错写实际的密钥名。
#[test]
fn an_input_secret_needs_0_8() {
    let manifest = json!({"secrets": {
        "brave_key": {"input": {"title": {"en": "Brave key"}}}
    }});
    let found = problems(&document("0.7", manifest.clone(), plain()));
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(
        found[0].contains("x-wonton.secrets.brave_key.input"),
        "{found:?}"
    );
    assert!(problems(&document("0.8", manifest, plain())).is_empty());
}

#[test]
fn role_fields_require_0_13() {
    for webui in [
        json!({"service": "web", "port": 80, "accounts": "per_user"}),
        json!({"service": "web", "port": 80, "oidc": {"redirect_paths": ["/cb"], "admin_groups": ["admin"]}}),
        json!({"service": "web", "port": 80, "oidc": {"redirect_paths": ["/cb"],
            "role_claim": {"name": "immich_role", "admin": "admin", "user": "user"}}}),
    ] {
        let manifest = json!({ "webui": webui });
        let found = problems(&document("0.12", manifest.clone(), plain()));
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(problems(&document("0.13", manifest, plain())).is_empty());
    }
}

#[test]
fn smtp_variables_require_spec_0_11_in_compose_and_inline_configs() {
    for name in ["WONTON_SMTP_HOST", "WONTON_SMTP_PORT", "WONTON_MAIL_FROM"] {
        for reference in [
            format!("${name}"),
            format!("${{{name}}}"),
            format!("${{{name}:?required}}"),
            format!("${{{name}?required}}"),
            format!("${{OTHER:-${{{name}}}}}"),
            format!("${{OTHER:+${{{name}}}}}"),
        ] {
            let manifest = json!({"permissions": ["notify"]});
            let mut doc = document("0.10", manifest, plain());
            doc["services"]["web"]["environment"] = json!({"SMTP": reference});
            assert_eq!(problems(&doc).len(), 1, "{doc}");
            doc["services"]["web"]["environment"] = json!([format!("SMTP={reference}")]);
            assert_eq!(problems(&doc).len(), 1, "{doc}");
            doc["services"]["web"]["environment"] = json!({});
            doc["services"]["web"]["configs"] = json!(["mail"]);
            doc["configs"] = json!({"mail": {"content": format!("smtp={reference}")}});
            assert_eq!(problems(&doc).len(), 1, "{doc}");
            doc["x-wonton"]["spec"] = json!("0.11");
            assert!(problems(&doc).is_empty(), "{doc}");
            doc["services"]["web"]["environment"] = json!({"SMTP": reference});
            assert!(problems(&doc).is_empty(), "{doc}");
        }
    }
}

#[test]
fn smtp_names_outside_interpolated_values_do_not_raise_the_floor() {
    let service = json!({
        "image": "nginx",
        "environment": {
            "WONTON_SMTP_HOST": "WONTON_SMTP_HOST",
            "${WONTON_SMTP_PORT}": "$${WONTON_SMTP_PORT}",
            "FROM": "$${WONTON_MAIL_FROM}",
            "OTHER": "${WONTON_SMTP_HOST_SUFFIX:-local}",
            "ERROR": "${OTHER:?use ${WONTON_MAIL_FROM}}"
        },
        "x-mail": "${WONTON_MAIL_FROM}"
    });
    let mut doc = document(
        "0.10",
        json!({"permissions": ["notify"], "description": {"en": "${WONTON_SMTP_HOST}"}}),
        service,
    );
    doc["configs"] = json!({"mail": {"file": "${WONTON_MAIL_FROM}.txt"}});
    assert!(problems(&doc).is_empty());
    for reference in [
        "${WONTON_SMTP_HOST:-smtp.example}",
        "${WONTON_SMTP_PORT-25}",
        "${WONTON_MAIL_FROM:-}",
        "${WONTON_SMTP_HOST:+enabled}",
        "${WONTON_MAIL_FROM+enabled}",
        "${WONTON_MATRIX_ALLOWED_USERS:-${WONTON_MATRIX_OWNER}}",
    ] {
        let service = json!({"image": "nginx", "environment": {"VALUE": reference}});
        let mut doc = document("0.8", json!({}), service);
        doc["configs"] = json!({"mail": {"content": reference}});
        assert!(problems(&doc).is_empty(), "{doc}");
    }
}

#[test]
fn all_new_variables_are_checked_even_after_an_unrelated_reference() {
    let service = json!({"image": "nginx", "environment": {"SMTP":
        "${OTHER} ${WONTON_SMTP_HOST}:${WONTON_SMTP_PORT} ${WONTON_MAIL_FROM} ${WONTON_SMTP_HOST}"
    }});
    assert_eq!(problems(&document("0.10", json!({}), service)).len(), 3);
}

#[test]
fn plugin_forms_require_spec_0_10() {
    for (field, manifest) in [
        ("x-wonton.runtime", json!({"runtime": "compose"})),
        (
            "x-wonton.agent.skills.mount",
            json!({"agent": {"skills": {"mount": "/opt/wonton/skills"}}}),
        ),
    ] {
        let found = problems(&document("0.9", manifest.clone(), plain()));
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(found[0].contains(field), "{found:?}");
        assert!(found[0].contains("spec \"0.10\""), "{found:?}");
        assert!(problems(&document("0.10", manifest, plain())).is_empty());
    }
}

#[test]
fn tunnel_ports_need_0_11() {
    let manifest = json!({"tunnel": {"ports": [{"service": "web", "port": 1024}]}});
    assert_eq!(
        problems(&document("0.10", manifest.clone(), plain())).len(),
        1
    );
    assert!(problems(&document("0.11", manifest, plain())).is_empty());
}

#[test]
fn proxy_role_templates_require_0_9_in_headers_and_cookies_only() {
    for kind in ["headers", "cookies"] {
        let manifest = json!({"webui": {"proxy": {(kind): {
            "identity": "prefix.{role}",
            "username": "{user}",
            "secret": "{secret:role}",
            "literal": "role"
        }}}});
        for version in ["0.7", "0.8"] {
            assert_eq!(
                problems(&document(version, manifest.clone(), plain())).len(),
                1
            );
        }
        assert!(problems(&document("0.9", manifest, plain())).is_empty());
    }
    let manifest = json!({"webui": {"proxy": {"headers": {
        "username": "{user}", "secret": "{secret:role}", "literal": "role"
    }}}});
    assert!(problems(&document("0.8", manifest, plain())).is_empty());
}

#[test]
fn capability_dependencies_need_spec_0_11_including_models() {
    for field in ["requires", "wants"] {
        for capability in ["models", "torrent_client", "x-acme.ledger"] {
            let manifest = json!({field: [capability]});
            assert_eq!(
                problems(&document("0.10", manifest.clone(), plain())).len(),
                1,
                "{field}: {capability}"
            );
            assert!(problems(&document("0.11", manifest, plain())).is_empty());
        }
        let manifest = json!({field: []});
        assert_eq!(
            problems(&document("0.10", manifest.clone(), plain())).len(),
            1
        );
        assert!(problems(&document("0.11", manifest, plain())).is_empty());
    }
}

#[test]
fn every_generic_provider_needs_spec_0_11_without_raising_legacy_provider_floors() {
    let mut manifest = json!({"provides": {
        "models": {"service": "web", "port": 80},
        "egress_proxy": {"service": "web", "port": 80}
    }});
    assert!(problems(&document("0.10", manifest.clone(), plain())).is_empty());
    for (index, capability) in ["torrent_client", "x-acme.ledger"].into_iter().enumerate() {
        manifest["provides"][capability] = json!({"service": "web", "port": 80});
        assert_eq!(
            problems(&document("0.10", manifest.clone(), plain())).len(),
            index + 1
        );
        assert!(problems(&document("0.11", manifest.clone(), plain())).is_empty());
    }
}

#[test]
fn dedicated_provider_fields_keep_their_original_spec_floors() {
    let models = json!({"provides": {"models": {"service": "web", "port": 80}}});
    assert!(problems(&document("0.6", models.clone(), plain())).is_empty());
    let mut with_commands = models;
    with_commands["provides"]["models"]["providers"] = json!({"list": ["list"]});
    let egress = json!({"provides": {"egress_proxy": {"service": "web", "port": 80}}});
    for manifest in [with_commands, egress] {
        assert_eq!(
            problems(&document("0.6", manifest.clone(), plain())).len(),
            1
        );
        assert!(problems(&document("0.7", manifest, plain())).is_empty());
    }
}
