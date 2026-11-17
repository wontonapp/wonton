use serde_json::{Value, json};

use super::{Added, TABLE, problems};
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

fn healthchecked() -> Value {
    json!({"image": "nginx", "healthcheck": {"test": ["CMD", "true"]}})
}

#[test]
fn a_newer_field_under_an_older_spec_names_the_field_and_the_version() {
    let manifest =
        json!({"webui": {"service": "web", "port": 80, "oidc": {"post_logout_paths": ["/bye"]}}});
    let found = problems(&document("0.6", manifest.clone(), healthchecked()));
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(
        found[0].contains("x-wonton.webui.oidc.post_logout_paths"),
        "{found:?}"
    );
    assert!(found[0].contains("\"0.7\""), "{found:?}");
    assert!(problems(&document("0.7", manifest, healthchecked())).is_empty());
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
fn a_license_url_needs_spec_0_14() {
    let manifest = json!({
        "license": "LicenseRef-Acme-Eula",
        "license_url": "https://example.com/eula",
    });
    let found = problems(&document("0.13", manifest.clone(), plain()));
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(found[0].contains("x-wonton.license_url"), "{found:?}");
    assert!(problems(&document("0.14", manifest, plain())).is_empty());
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
    assert!(problems(&document("0.5", manifest, healthchecked())).is_empty());
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
        let found = problems(&document("0.12", manifest.clone(), healthchecked()));
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(problems(&document("0.13", manifest, healthchecked())).is_empty());
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
fn the_gateway_cidr_and_app_host_variables_require_spec_0_14() {
    for (name, reference) in [
        ("WONTON_GATEWAY_CIDR", "TRUST: ${WONTON_GATEWAY_CIDR}"),
        ("WONTON_APP_HOST", "HOSTS: $WONTON_APP_HOST"),
    ] {
        let mut doc = document("0.13", json!({}), plain());
        doc["services"]["web"]["environment"] = json!({"VALUE": reference});
        let found = problems(&doc);
        assert_eq!(found.len(), 1, "{name}: {found:?}");
        doc["x-wonton"]["spec"] = json!("0.14");
        assert!(problems(&doc).is_empty(), "{name}");
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
#[test]
fn base64url_secret_encoding_needs_spec_0_14() {
    let manifest =
        json!({"secrets": {"key": {"generate": {"bytes": 32, "encoding": "base64url"}}}});
    let found = problems(&document("0.13", manifest.clone(), plain()));
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(
        found[0].contains("x-wonton.secrets.key.generate.encoding: base64url"),
        "{found:?}"
    );
}

#[test]
fn a_widget_needs_spec_0_14() {
    let manifest =
        json!({"webui": {"service": "web", "port": 80}, "matrix_widget": {"url": "/room"}});
    let found = problems(&document("0.13", manifest.clone(), healthchecked()));
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(found[0].contains("x-wonton.matrix_widget"), "{found:?}");
    assert!(problems(&document("0.14", manifest, healthchecked())).is_empty());
}

#[test]
fn clients_needs_spec_0_14() {
    let manifest = json!({
        "permissions": ["network.clients"],
        "clients": {"tcp": [{"service": "web", "port": 22, "name": "ssh"}]}
    });
    let found = problems(&document("0.13", manifest.clone(), plain()));
    assert!(
        found.iter().any(|p| p.contains("x-wonton.clients")),
        "{found:?}"
    );
    assert!(
        found
            .iter()
            .any(|p| p.contains("permission network.clients")),
        "{found:?}"
    );
    assert_eq!(found.len(), 2, "{found:?}");
    assert!(problems(&document("0.14", manifest, plain())).is_empty());
}

#[test]
fn the_install_deadline_needs_spec_0_14() {
    let manifest = json!({"install_timeout_secs": 900});
    let found = problems(&document("0.13", manifest.clone(), plain()));
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(
        found[0].contains("x-wonton.install_timeout_secs"),
        "{found:?}"
    );
    assert!(problems(&document("0.14", manifest, plain())).is_empty());
}

#[test]
fn guests_needs_spec_0_14() {
    let manifest = json!({"webui": {"service": "web", "port": 80, "guests": true}});
    let found = problems(&document("0.13", manifest.clone(), healthchecked()));
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(found[0].contains("x-wonton.webui.guests"), "{found:?}");
    assert!(problems(&document("0.14", manifest, healthchecked())).is_empty());
}

#[test]
fn other_secret_encodings_stay_at_the_old_spec() {
    let manifest = json!({"secrets": {"key": {"generate": {"encoding": "base64"}}}});
    assert!(problems(&document("0.13", manifest, plain())).is_empty());
}

#[test]
fn the_supported_spec_version_covers_the_newest_table_entry() {
    assert!(TABLE.iter().all(|(since, _)| *since <= SPEC_VERSION));
}

#[test]
fn external_url_needs_spec_0_14() {
    let manifest = json!({"clients": {"tcp": [{"service": "web", "port": 22, "name": "ssh"}]}});
    let service = json!({"image": "nginx", "labels": {"url": "${WONTON_EXTERNAL_URL}"}});
    let found = problems(&document("0.13", manifest.clone(), service.clone()));
    assert!(
        found
            .iter()
            .any(|p| p.contains("services.web.labels.url references WONTON_EXTERNAL_URL")),
        "{found:?}"
    );
    assert!(problems(&document("0.14", manifest, service)).is_empty());
}

#[test]
fn devices_and_the_disk_permission_require_0_14() {
    for manifest in [
        json!({"devices": {"serial": [{"service": "z", "target": "/dev/z"}]}}),
        json!({"permissions": ["devices.disk"]}),
    ] {
        let found = problems(&document("0.13", manifest.clone(), plain()));
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(problems(&document("0.14", manifest, plain())).is_empty());
    }
}

#[test]
fn remote_mcp_needs_0_15() {
    let manifest = json!({"remote_mcp": {"url": "https://mcp.example.com/mcp"}});
    let found = problems(&document("0.14", manifest.clone(), plain()));
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(found[0].contains("x-wonton.remote_mcp"), "{found:?}");
    assert!(found[0].contains("0.15"), "{found:?}");
    assert!(problems(&document("0.15", manifest, plain())).is_empty());
}

#[test]
fn automatic_webui_health_requires_spec_0_15_without_an_effective_compose_check() {
    let manifest = json!({"webui": {"service": "web", "port": 80}});
    for check in [
        Value::Null,
        json!({"disable": true, "test": ["CMD", "true"]}),
        json!({"test": ["NONE"]}),
        json!({"test": []}),
        json!({"test": ""}),
        json!({"interval": "30s"}),
    ] {
        let mut service = plain();
        service["healthcheck"] = check;
        assert_eq!(
            problems(&document("0.14", manifest.clone(), service.clone())).len(),
            1
        );
        assert!(problems(&document("0.15", manifest.clone(), service)).is_empty());
    }
}

#[test]
fn existing_compose_and_explicit_webui_health_keep_their_spec_floors() {
    let manifest = json!({"webui": {"service": "web", "port": 80}});
    for test in [json!(["CMD", "true"]), json!("true")] {
        let mut service = plain();
        service["healthcheck"] = json!({"test": test});
        assert!(problems(&document("0.6", manifest.clone(), service)).is_empty());
    }
    let manifest = json!({"webui": {"service": "web", "port": 80, "health": "/health"}});
    assert_eq!(
        problems(&document("0.12", manifest.clone(), plain())).len(),
        1
    );
    assert!(problems(&document("0.13", manifest, plain())).is_empty());
}

#[test]
fn automatic_webui_health_only_counts_resident_compose_web_services() {
    let manifest = json!({"webui": {"service": "web", "port": 80}});
    for runtime in ["static_web", "link", "remote_mcp"] {
        let mut manifest = manifest.clone();
        manifest["runtime"] = json!(runtime);
        assert!(problems(&document("0.14", manifest, plain())).is_empty());
    }
    let mut one_shot = plain();
    one_shot["restart"] = json!("no");
    assert!(problems(&document("0.14", manifest.clone(), one_shot)).is_empty());
    let mut doc = document("0.14", manifest.clone(), plain());
    doc["services"]["app"] = json!({
        "image": "nginx",
        "depends_on": {"web": {"condition": "service_completed_successfully"}}
    });
    assert!(problems(&doc).is_empty());
    doc["services"] = json!({"sidecar": {"image": "nginx"}});
    assert!(problems(&doc).is_empty());
    let mut doc = document("0.14", manifest, plain());
    doc["x-wonton"]["runtime"] = json!("compose");
    assert_eq!(problems(&doc).len(), 1);
}
/// `WONTON_BOT_AI_LEVEL` 是 0.15 才开始注入的变量（#615）：`spec` 写低了要在 `plugin check` 就报
/// 要升 spec，而不是装到旧核心上才在插值时报“变量没有注入”。
#[test]
fn the_bot_ai_level_variable_requires_spec_0_15() {
    let service = json!({"image": "nginx", "environment": {"LEVEL": "${WONTON_BOT_AI_LEVEL}"}});
    let found = problems(&document(
        "0.14",
        json!({"permissions": ["matrix.bot"]}),
        service.clone(),
    ));
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(
        found[0].contains("services.web.environment.LEVEL references WONTON_BOT_AI_LEVEL"),
        "{found:?}"
    );
    assert!(found[0].contains("spec \"0.15\""), "{found:?}");
    assert!(problems(&document("0.15", json!({}), service)).is_empty());
}

/// 每个 Bot 的变量也是 0.9 的写法：0.9 起每个 Bot 一组容器，旧核心不注入。
#[test]
fn per_bot_variables_require_spec_0_9() {
    for name in ["WONTON_BOT_NAME", "WONTON_BOT_DATA"] {
        let service = json!({"image": "nginx", "environment": {"V": format!("${{{name}}}")}});
        let mut doc = document("0.8", json!({"bots": {"services": ["web"]}}), service);
        assert_eq!(problems(&doc).len(), 1, "{name}: {doc}");
        doc["x-wonton"]["spec"] = json!("0.9");
        assert!(problems(&doc).is_empty(), "{name}: {doc}");
    }
}

/// 规范 0.12 的 `x-wonton.settings`：表里一直没有，低 `spec` 的清单不报。
#[test]
fn settings_need_spec_0_12() {
    let manifest = json!({"settings": {"budget": {"type": "number"}}});
    let found = problems(&document("0.11", manifest.clone(), plain()));
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(found[0].contains("x-wonton.settings"), "{found:?}");
    assert!(problems(&document("0.12", manifest, plain())).is_empty());
}

/// 规范 0.15 的 `agent_acp`：`provides` 里的声明不按通用能力的 0.11 报，只报 0.15；
/// `requires`/`wants` 认数组里的成员；别的通用能力照旧 0.11。
#[test]
fn agent_acp_needs_spec_0_15() {
    let provides = json!({"provides": {"agent_acp": {"service": "web", "port": 8080}}});
    let found = problems(&document("0.10", provides.clone(), plain()));
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(
        found[0].contains("x-wonton.provides.agent_acp"),
        "{found:?}"
    );
    assert!(found[0].contains("spec \"0.15\""), "{found:?}");
    assert!(problems(&document("0.15", provides, plain())).is_empty());
    for field in ["requires", "wants"] {
        let manifest = json!({field: ["agent_acp"]});
        let found = problems(&document("0.14", manifest.clone(), plain()));
        assert_eq!(found.len(), 1, "{field}: {found:?}");
        assert!(
            found[0].contains(&format!("x-wonton.{field}: agent_acp")),
            "{found:?}"
        );
        assert!(problems(&document("0.15", manifest, plain())).is_empty());
    }
    let other = json!({
        "requires": ["torrent_client"],
        "provides": {"torrent_client": {"service": "web", "port": 8080}}
    });
    assert!(problems(&document("0.11", other, plain())).is_empty());
}

/// 表里一行的名字，照规范“各版本新增的字段”的措辞起，好对着抄。
fn label(added: &Added) -> String {
    let path = |path: &[&str]| {
        path.iter()
            .map(|part| if *part == "*" { "<名字>" } else { *part })
            .collect::<Vec<_>>()
            .join(".")
    };
    match added {
        Added::Field(pathed) => format!("x-wonton.{}", path(pathed)),
        Added::Value(field, value) => format!("x-wonton.{field}: {value}"),
        Added::ValueAt(pathed, value) => format!("x-wonton.{}: {value}", path(pathed)),
        Added::GenericProvides => "x-wonton.provides.<能力>".to_owned(),
        Added::Permission(name) => format!("permission {name}"),
        Added::ServiceKey(key) => format!("services {key}"),
        Added::Variable(name) => format!("variable {name}"),
        Added::NetworkModeService => "network_mode: service:<服务名>".to_owned(),
        Added::NoNewPrivilegesWithoutPermission => {
            "security_opt no-new-privileges 不用 security.opt".to_owned()
        }
        Added::ProxyRole => "代登录模板 {role}".to_owned(),
        Added::AutomaticWebuiHealth => "无健康检查的 webui 服务由核心代探测".to_owned(),
    }
}

/// 照 wonton-docs 的 plugin-spec.md“各版本新增的字段”手抄的期望清单：表一改这里就红，提醒
/// 同步改这份清单和规范的对应版本节（规范在另一个仓库，测试读不到）。
///
/// 有意不进表的（规范那一节写了、这里没有）：
/// - `webui.oidc.owner_groups`（0.7）、`webui.friends`（0.9）：0.13 已删，现代核心按未知字段拒绝，
///   加下限只会多报一条误导的“升到 0.7/0.9”。
/// - `WONTON_PERMISSION_*`（0.8）：名字带变量没法精确匹配；引用它的清单必然声明了
///   `optional_permissions`（0.8 的字段），由那条间接拦住。
/// - `WONTON_MATRIX_ALLOWED_USERS`（0.9 改名）：规范让作者用
///   `${WONTON_MATRIX_ALLOWED_USERS:-${WONTON_MATRIX_OWNER}}` 兼容旧核心、`spec` 保持旧版，
///   加下限会把这条官方兼容写法误报出来。
/// - `WONTON_OWNER_EMAIL` 和 `{email}` 的取值（0.11）：一律内部邮箱是语义收紧，不是新写法。
/// - `requirements.gpu.vendors` 的顺序（0.14）：行为约束，不是清单语法。
/// - 确认卡片的 `origin`（0.13）：规范明说是调用时的写法，清单不必改 `spec`。
/// - `WONTON_SHARE_<名称>`（0.9）：名字带变量没法精确匹配，声明它的 `webui.friends` 又已删，
///   两头都拦不住，是已知缺口。
const EXPECTED: &[(&str, &str)] = &[
    // 0.7
    ("0.7", "x-wonton.webui.cross_origin_isolated"),
    ("0.7", "x-wonton.webui.oidc.post_logout_paths"),
    ("0.7", "x-wonton.mcp.headers"),
    ("0.7", "x-wonton.mcp.tools"),
    ("0.7", "x-wonton.bot.avatars"),
    ("0.7", "x-wonton.bot.chat_layout"),
    ("0.7", "x-wonton.provides.egress_proxy"),
    ("0.7", "x-wonton.provides.models.providers"),
    ("0.7", "permission egress.proxy"),
    ("0.7", "permission plugins.health"),
    ("0.7", "services mem_limit"),
    ("0.7", "services mem_reservation"),
    ("0.7", "services cpus"),
    ("0.7", "services cpu_shares"),
    ("0.7", "services pids_limit"),
    ("0.7", "services cap_drop"),
    ("0.7", "network_mode: service:<服务名>"),
    ("0.7", "security_opt no-new-privileges 不用 security.opt"),
    // 0.8
    ("0.8", "x-wonton.optional_permissions"),
    ("0.8", "x-wonton.secrets.<名字>.show_owner"),
    ("0.8", "x-wonton.secrets.<名字>.input"),
    // 0.9
    ("0.9", "代登录模板 {role}"),
    ("0.9", "variable WONTON_BOT_NAME"),
    ("0.9", "variable WONTON_BOT_DATA"),
    // 0.10
    ("0.10", "x-wonton.runtime"),
    ("0.10", "x-wonton.agent.skills.mount"),
    // 0.11
    ("0.11", "x-wonton.tunnel.ports"),
    ("0.11", "x-wonton.setup"),
    ("0.11", "x-wonton.requires"),
    ("0.11", "x-wonton.wants"),
    ("0.11", "x-wonton.provides.<能力>"),
    ("0.11", "variable WONTON_SMTP_HOST"),
    ("0.11", "variable WONTON_SMTP_PORT"),
    ("0.11", "variable WONTON_MAIL_FROM"),
    // 0.12
    ("0.12", "x-wonton.settings"),
    ("0.12", "x-wonton.runtime: static_web"),
    ("0.12", "x-wonton.runtime: link"),
    ("0.12", "x-wonton.static_web"),
    ("0.12", "x-wonton.link"),
    ("0.12", "x-wonton.provides.<名字>.mount"),
    // 0.13
    ("0.13", "x-wonton.webui.accounts"),
    ("0.13", "x-wonton.webui.oidc.admin_groups"),
    ("0.13", "x-wonton.webui.oidc.role_claim"),
    ("0.13", "x-wonton.webui.health"),
    // 0.14
    ("0.14", "x-wonton.install_timeout_secs"),
    (
        "0.14",
        "x-wonton.secrets.<名字>.generate.encoding: base64url",
    ),
    ("0.14", "variable WONTON_GATEWAY_CIDR"),
    ("0.14", "variable WONTON_APP_HOST"),
    ("0.14", "x-wonton.matrix_widget"),
    ("0.14", "x-wonton.license_url"),
    ("0.14", "x-wonton.gpu.images"),
    ("0.14", "variable WONTON_GPU_VENDOR"),
    ("0.14", "x-wonton.clients"),
    ("0.14", "x-wonton.webui.guests"),
    ("0.14", "permission network.clients"),
    ("0.14", "variable WONTON_EXTERNAL_URL"),
    ("0.14", "x-wonton.devices"),
    ("0.14", "permission devices.disk"),
    // 0.15
    ("0.15", "x-wonton.remote_mcp"),
    ("0.15", "无健康检查的 webui 服务由核心代探测"),
    ("0.15", "x-wonton.provides.agent_acp"),
    ("0.15", "x-wonton.requires: agent_acp"),
    ("0.15", "x-wonton.wants: agent_acp"),
    ("0.15", "variable WONTON_AGENT_ACP_URL"),
    ("0.15", "variable WONTON_BOT_AI_LEVEL"),
];

#[test]
fn the_table_matches_the_expected_per_version_list() {
    let generated: Vec<(String, String)> = TABLE
        .iter()
        .map(|(since, added)| (since.to_string(), label(added)))
        .collect();
    let expected: Vec<(String, String)> = EXPECTED
        .iter()
        .map(|(since, name)| (since.to_string(), name.to_string()))
        .collect();
    assert_eq!(
        generated, expected,
        "spec_floor 表和期望清单不一致：先改这份清单，再同步 wonton-docs 的 plugin-spec.md“各版本新增的字段”"
    );
}
