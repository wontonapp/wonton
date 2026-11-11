//! 每一版规范新增的字段和变量（插件规范“各版本新增的字段”一节照这张表写）：清单用了新写法、`spec`
//! 却写得比那一版低时，`plugin check` 报不合格。
//!
//! 只在 `plugin check` 那份校验里查（Agent 经 MCP 装自己写的插件包也走它），按 ID 或从商店装不查。
//! `spec` 写对了，旧核心先看版本号，就能提示升级，而不是在解析字段或插值时失败。

use std::collections::BTreeMap;
use std::fmt::Write;

use serde_json::Value;

use super::interpolate;
use super::store_info::SpecVersion;

/// 清单里一处新增的写法。
enum Added {
    /// `x-wonton` 下的一个字段，按路径找；路径里的 `"*"` 匹配所在表里的任意一个键，报错写实际的键
    /// （如按名字的表 `secrets.<名字>`）。
    Field(&'static [&'static str]),
    /// `x-wonton.permissions` 里的一项。
    Permission(&'static str),
    /// 某个服务里的一个 compose 字段。
    ServiceKey(&'static str),
    /// 服务或内联配置引用的注入变量。
    Variable(&'static str),
    /// `network_mode: "service:<服务名>"`。
    NetworkModeService,
    /// `security_opt` 里的 `no-new-privileges` 不用申请 `security.opt`。
    NoNewPrivilegesWithoutPermission,
    /// 网页代登录模板里的 `{role}`。
    ProxyRole,
}

const V0_7: SpecVersion = SpecVersion::new(0, 7);
const V0_8: SpecVersion = SpecVersion::new(0, 8);
const V0_9: SpecVersion = SpecVersion::new(0, 9);
const V0_10: SpecVersion = SpecVersion::new(0, 10);
const V0_11: SpecVersion = SpecVersion::new(0, 11);

/// 0.6 以后才有的写法。新加清单字段时在这里加一行，规范的“各版本新增的字段”同步加。
const TABLE: &[(SpecVersion, Added)] = &[
    (V0_7, Added::Field(&["webui", "cross_origin_isolated"])),
    (V0_7, Added::Field(&["webui", "oidc", "post_logout_paths"])),
    (V0_7, Added::Field(&["webui", "oidc", "owner_groups"])),
    (V0_7, Added::Field(&["mcp", "headers"])),
    (V0_7, Added::Field(&["mcp", "tools"])),
    (V0_7, Added::Field(&["bot", "avatars"])),
    (V0_7, Added::Field(&["bot", "chat_layout"])),
    (V0_7, Added::Field(&["provides", "egress_proxy"])),
    (V0_7, Added::Field(&["provides", "models", "providers"])),
    (V0_7, Added::Permission("egress.proxy")),
    (V0_7, Added::Permission("plugins.health")),
    (V0_7, Added::ServiceKey("mem_limit")),
    (V0_7, Added::ServiceKey("mem_reservation")),
    (V0_7, Added::ServiceKey("cpus")),
    (V0_7, Added::ServiceKey("cpu_shares")),
    (V0_7, Added::ServiceKey("pids_limit")),
    (V0_7, Added::ServiceKey("cap_drop")),
    (V0_7, Added::NetworkModeService),
    (V0_7, Added::NoNewPrivilegesWithoutPermission),
    (V0_8, Added::Field(&["optional_permissions"])),
    (V0_8, Added::Field(&["secrets", "*", "show_owner"])),
    (V0_8, Added::Field(&["secrets", "*", "input"])),
    (V0_9, Added::Field(&["webui", "friends"])),
    (V0_9, Added::ProxyRole),
    (V0_10, Added::Field(&["runtime"])),
    (V0_10, Added::Field(&["agent", "skills", "mount"])),
    (V0_11, Added::Field(&["tunnel", "ports"])),
    (V0_11, Added::Field(&["setup"])),
    (V0_11, Added::Variable("WONTON_SMTP_HOST")),
    (V0_11, Added::Variable("WONTON_SMTP_PORT")),
    (V0_11, Added::Variable("WONTON_MAIL_FROM")),
];

/// 清单（整份 compose 文档）里用到、但 `spec` 写得太低的写法，一处一条。`spec` 读不出来时不查：解析那一步已经报过。
pub fn problems(document: &Value) -> Vec<String> {
    let manifest = &document["x-wonton"];
    let Some(declared) = manifest["spec"].as_str().and_then(SpecVersion::parse) else {
        return Vec::new();
    };
    let services: Vec<(&String, &Value)> = document["services"]
        .as_object()
        .map(|services| services.iter().collect())
        .unwrap_or_default();
    let permissions: Vec<&str> = manifest["permissions"]
        .as_array()
        .map(|list| list.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    let mut variables = variable_places(document, declared);
    let mut problems = Vec::new();
    for (since, added) in TABLE.iter().filter(|(since, _)| declared < *since) {
        let found = match added {
            Added::Variable(name) => variables
                .remove(name)
                .unwrap_or_default()
                .into_iter()
                .map(|place| format!("{place} references {name}"))
                .collect(),
            _ => places(added, manifest, &services, &permissions),
        };
        for place in found {
            problems.push(format!(
                "{place} needs spec \"{since}\" or later, but the manifest says spec \"{declared}\"; \
                 raise spec so that older home servers say \"update the home server\" instead of \"unknown field\""
            ));
        }
    }
    problems
}

/// 清单里用到这处写法的地方，用来写进报错。
fn places(
    added: &Added,
    manifest: &Value,
    services: &[(&String, &Value)],
    permissions: &[&str],
) -> Vec<String> {
    let in_services = |found: &dyn Fn(&Value) -> bool, what: &str| {
        services
            .iter()
            .filter(|(_, service)| found(service))
            .map(|(name, _)| format!("services.{name}.{what}"))
            .collect()
    };
    match added {
        Added::Field(path) => {
            let mut found = Vec::new();
            walk(manifest, path, &mut String::from("x-wonton"), &mut found);
            found
        }
        Added::ProxyRole => ["headers", "cookies"]
            .into_iter()
            .flat_map(|kind| {
                manifest["webui"]["proxy"][kind]
                    .as_object()
                    .into_iter()
                    .flatten()
                    .filter(|(_, value)| value.as_str().is_some_and(|v| v.contains("{role}")))
                    .map(move |(name, _)| format!("x-wonton.webui.proxy.{kind}.{name}"))
            })
            .collect(),
        Added::Permission(name) => permissions
            .contains(name)
            .then(|| format!("permission {name}"))
            .into_iter()
            .collect(),
        Added::ServiceKey(key) => in_services(&|service| !service[key].is_null(), key),
        Added::NetworkModeService => in_services(
            &|service| {
                service["network_mode"]
                    .as_str()
                    .is_some_and(|mode| mode.starts_with("service:"))
            },
            "network_mode: \"service:…\"",
        ),
        Added::NoNewPrivilegesWithoutPermission if !permissions.contains(&"security.opt") => {
            in_services(
                &|service| {
                    service["security_opt"].as_array().is_some_and(|opts| {
                        opts.iter()
                            .filter_map(Value::as_str)
                            .any(|opt| opt.starts_with("no-new-privileges"))
                    })
                },
                "security_opt: no-new-privileges without the security.opt permission",
            )
        }
        Added::NoNewPrivilegesWithoutPermission => Vec::new(),
        Added::Variable(_) => unreachable!("variable references are scanned together"),
    }
}

/// 走完一条字段路径，用到的写进 `found`：`place` 是走到这里的报错位置，`"*"` 展开成所在表里的每个键。
fn walk(value: &Value, path: &[&str], place: &mut String, found: &mut Vec<String>) {
    let Some((key, rest)) = path.split_first() else {
        if !value.is_null() {
            found.push(place.clone());
        }
        return;
    };
    let mut descend = |key: &str, next: &Value| {
        let length = place.len();
        place.push('.');
        place.push_str(key);
        walk(next, rest, place, found);
        place.truncate(length);
    };
    if *key == "*" {
        if let Some(table) = value.as_object() {
            for (name, next) in table {
                descend(name, next);
            }
        }
    } else if let Some(next) = value.get(key).filter(|next| !next.is_null()) {
        descend(key, next);
    }
}

fn variable_places(document: &Value, declared: SpecVersion) -> BTreeMap<&'static str, Vec<String>> {
    let mut variables: BTreeMap<_, _> = TABLE
        .iter()
        .filter_map(|(since, added)| match added {
            Added::Variable(name) if declared < *since => Some((*name, Vec::new())),
            _ => None,
        })
        .collect();
    if variables.is_empty() {
        return variables;
    }
    if let Some(services) = document["services"].as_object() {
        for (name, service) in services {
            if let Some(fields) = service.as_object() {
                for (key, value) in fields.iter().filter(|(key, _)| !key.starts_with("x-")) {
                    scan_variables(value, &mut format!("services.{name}.{key}"), &mut variables);
                }
            }
        }
    }
    if let Some(configs) = document["configs"].as_object() {
        for (name, config) in configs {
            if let Some(content) = config.get("content") {
                scan_variables(
                    content,
                    &mut format!("configs.{name}.content"),
                    &mut variables,
                );
            }
        }
    }
    variables
}

fn scan_variables(
    value: &Value,
    place: &mut String,
    variables: &mut BTreeMap<&'static str, Vec<String>>,
) {
    match value {
        Value::String(text) => {
            // 插值语法由渲染检查报告；这里保留语法错误之前已识别的引用。
            let _ = interpolate::references(text, &mut |name| {
                if let Some(found) = variables.get_mut(name)
                    && found
                        .last()
                        .is_none_or(|last| last.as_str() != place.as_str())
                {
                    found.push(place.clone());
                }
            });
        }
        Value::Array(items) => {
            for (i, item) in items.iter().enumerate() {
                let mark = place.len();
                let _ = write!(place, "[{i}]");
                scan_variables(item, place, variables);
                place.truncate(mark);
            }
        }
        Value::Object(fields) => {
            for (key, item) in fields {
                let mark = place.len();
                place.push('.');
                place.push_str(key);
                scan_variables(item, place, variables);
                place.truncate(mark);
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
}

#[cfg(test)]
#[path = "spec_floor_tests.rs"]
mod tests;
