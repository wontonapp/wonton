//! 每一版规范新增的清单字段（插件规范“各版本新增的字段”一节照这张表写）：清单用到某一版才有的字段、`spec`
//! 却写得比那一版低时，`plugin check` 报不合格。
//!
//! 只在 `plugin check` 那份校验里查（Agent 经 MCP 装自己写的插件包也走它），按 ID 或从商店装不查：这份核心读得懂
//! 这些字段，装上能用；要防的是旧核心读到它们时报“未知字段”，而不是如实说“先升级核心”。`spec` 写对了，旧核心先看
//! 版本号，就会这样说。

use serde_json::Value;

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
    /// `network_mode: "service:<服务名>"`。
    NetworkModeService,
    /// `security_opt` 里的 `no-new-privileges` 不用申请 `security.opt`。
    NoNewPrivilegesWithoutPermission,
}

const V0_7: SpecVersion = SpecVersion::new(0, 7);
const V0_8: SpecVersion = SpecVersion::new(0, 8);

/// 0.6 以后才有的写法。新加清单字段时在这里加一行，规范的“各版本新增的字段”同步加。
const TABLE: &[(SpecVersion, Added)] = &[
    (V0_7, Added::Field(&["webui", "cross_origin_isolated"])),
    (V0_7, Added::Field(&["webui", "oidc", "post_logout_paths"])),
    (V0_7, Added::Field(&["webui", "oidc", "owner_groups"])),
    (V0_7, Added::Field(&["webui", "oidc", "requires_email"])),
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
    let mut problems = Vec::new();
    for (since, added) in TABLE.iter().filter(|(since, _)| declared < *since) {
        for place in places(added, manifest, &services, &permissions) {
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

#[cfg(test)]
#[path = "spec_floor_tests.rs"]
mod tests;
