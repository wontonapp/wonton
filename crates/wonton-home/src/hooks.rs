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
    let store = app.store.lock().await;
    let Some(home) = store.data.homes.get(label) else {
        return reject("unknown home");
    };
    if home.suspended || home.deleted || !matches(token, &home.token_hash) {
        return reject("home disabled or token revoked");
    }
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
    if home.usage_bytes >= 100_000_000_000 {
        return reject("monthly quota exceeded");
    }
    if request.op == "NewUserConn" && runtime.connections.get(label).copied().unwrap_or(0) >= 100 {
        return reject("connection quota exceeded");
    }
    if request.op == "NewProxy" {
        let content = &request.content;
        let Some(domains) = content["custom_domains"].as_array() else {
            return reject("missing custom domain");
        };
        let Some(domain) = domains.first().and_then(Value::as_str) else {
            return reject("invalid domain");
        };
        let Some(name) = content["proxy_name"].as_str() else {
            return reject("missing proxy name");
        };
        if content["proxy_type"] != "https"
            || domains.len() != 1
            || !allowed_domain(domain, label, &app.config.zone)
            || content["subdomain"].as_str().is_some_and(|s| !s.is_empty())
            || content["group"].as_str().is_some_and(|s| !s.is_empty())
            || !name
                .strip_prefix(&format!("{label}."))
                .is_some_and(valid_label)
        {
            return reject("proxy domain or type forbidden");
        }
        if !runtime.proxies.contains_key(name)
            && runtime
                .proxies
                .values()
                .filter(|owner| owner.as_str() == label)
                .count()
                >= 5
        {
            return reject("proxy quota exceeded");
        }
        runtime.proxies.insert(name.to_owned(), label.to_owned());
        runtime.proxy_reservations.insert(name.to_owned(), now());
        request.content["bandwidth_limit"] = json!("5MB");
        request.content["bandwidth_limit_mode"] = json!("server");
        return Json(json!({"reject":false,"unchange":false,"content":request.content}));
    }
    Json(json!({"reject":false,"unchange":true}))
}
