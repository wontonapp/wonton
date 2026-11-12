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
