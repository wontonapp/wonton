use crate::{
    config::valid_label,
    service::App,
    store::{matches, now},
};
use axum::{Json, extract::State};
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;

#[derive(Deserialize)]
pub struct Hook {
    op: String,
    content: Value,
}

fn reject(reason: &str) -> Json<Value> {
    Json(json!({"reject":true,"reject_reason":reason,"unchange":true}))
}

pub fn allowed_domain(domain: &str, home: &str, zone: &str) -> bool {
    let apex = format!("{home}.{zone}");
    domain == apex
        || domain
            .strip_suffix(&format!(".{apex}"))
            .is_some_and(|label| label != "lan" && valid_label(label))
}

/// A home may register its apex or its apex plus one wildcard, so one proxy
/// covers every plugin host without letting the home claim a sibling's name.
pub(crate) fn home_domains_ok(domains: &[Value], label: &str, zone: &str) -> bool {
    let apex = format!("{label}.{zone}");
    let wildcard = format!("*.{apex}");
    matches!(domains.len(), 1 | 2)
        && domains.iter().all(|domain| {
            domain
                .as_str()
                .is_some_and(|d| d == wildcard || allowed_domain(d, label, zone))
        })
        && (domains.len() == 1
            || domains
                .iter()
                .any(|domain| domain.as_str() == Some(wildcard.as_str())))
}

/// Returns the proxy name when a `NewProxy` request may be admitted, or `None`
/// when its type, domains, or name are outside the caller's allowlist.
fn proxy_name<'a>(app: &App, label: &str, is_system: bool, content: &'a Value) -> Option<&'a str> {
    let domains = content["custom_domains"].as_array()?;
    let name = content["proxy_name"].as_str()?;
    let proxy_type = content["proxy_type"].as_str();
    let domains_ok = if is_system {
        let allowlist = app.config.system.as_ref().map(|system| &system.domains);
        !domains.is_empty()
            && matches!(proxy_type, Some("http" | "https"))
            && domains.iter().all(|domain| {
                domain
                    .as_str()
                    .is_some_and(|d| allowlist.is_some_and(|a| a.iter().any(|x| x.as_str() == d)))
            })
    } else {
        proxy_type == Some("https") && home_domains_ok(domains, label, &app.config.zone)
    };
    if !domains_ok
        || content["subdomain"].as_str().is_some_and(|s| !s.is_empty())
        || content["group"].as_str().is_some_and(|s| !s.is_empty())
        || !name
            .strip_prefix(&format!("{label}."))
            .is_some_and(valid_label)
    {
        return None;
    }
    Some(name)
}

pub async fn handle(State(app): State<Arc<App>>, Json(mut request): Json<Hook>) -> Json<Value> {
    let login = request.op == "Login";
    if !matches!(
        request.op.as_str(),
        "Login" | "NewProxy" | "NewUserConn" | "Ping" | "CloseProxy"
    ) {
        return reject("unsupported hook");
    }
    let user = if login {
        &request.content
    } else {
        &request.content["user"]
    };
    let Some(label) = user["user"].as_str() else {
        return reject("missing home");
    };
    let Some(token) = user["metas"]["token"].as_str() else {
        return reject("missing token");
    };
    // `system` is an operator-owned frpc identity for the fixed public vhosts
    // (push, relay, omarchy, names, apex); every other user must be a home row.
    let is_system = label == "system";
    let store = app.store.lock().await;
    let home = if is_system {
        let Some(system) = app.config.system.as_ref() else {
            return reject("system access not configured");
        };
        if !matches(token, &system.token_hash) {
            return reject("system token mismatch");
        }
        None
    } else {
        let Some(home) = store.data.homes.get(label) else {
            return reject("unknown home");
        };
        if home.suspended || home.deleted || !matches(token, &home.token_hash) {
            return reject("home disabled or token revoked");
        }
        Some(home)
    };
    let mut runtime = app.runtime.lock().await;
    if request.op == "CloseProxy" {
        if let Some(name) = request.content["proxy_name"].as_str() {
            if !name.starts_with(&format!("{label}.")) {
                return reject("proxy not owned");
            }
            runtime.proxies.remove(name);
            runtime.proxy_reservations.remove(name);
        }
        return Json(json!({"reject":false,"unchange":true}));
    }
    if !login && now().saturating_sub(runtime.metrics_at) > 65 {
        return reject("relay quota metrics unavailable");
    }
    if let Some(home) = home {
        if home.usage_bytes >= 100_000_000_000 {
            return reject("monthly quota exceeded");
        }
        if request.op == "NewUserConn"
            && runtime.connections.get(label).copied().unwrap_or(0) >= 100
        {
            return reject("connection quota exceeded");
        }
    }
    if request.op == "NewProxy" {
        let content = &request.content;
        let Some(name) = proxy_name(&app, label, is_system, content) else {
            return reject("proxy domain or type forbidden");
        };
        let quota = if is_system { 16 } else { 5 };
        if !runtime.proxies.contains_key(name)
            && runtime
                .proxies
                .values()
                .filter(|owner| owner.as_str() == label)
                .count()
                >= quota
        {
            return reject("proxy quota exceeded");
        }
        runtime.proxies.insert(name.to_owned(), label.to_owned());
        runtime.proxy_reservations.insert(name.to_owned(), now());
        request.content["bandwidth_limit"] = json!(if is_system { "20MB" } else { "5MB" });
        request.content["bandwidth_limit_mode"] = json!("server");
        return Json(json!({"reject":false,"unchange":false,"content":request.content}));
    }
    Json(json!({"reject":false,"unchange":true}))
}
