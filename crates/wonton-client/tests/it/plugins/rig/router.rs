//! 假核心的路由：记下每个请求，按预设、按住、引导和资料接口回，其余走默认。

use std::sync::Arc;

use axum::body::{Body, to_bytes};
use axum::extract::{Request, State};
use axum::http::{Method, StatusCode, header};
use axum::response::{IntoResponse, Response};
use wonton_client::{InstalledPlugins, PluginHealth, PluginState};

use super::answers::{Canned, Headered, core_error, core_error_response, json_value};
use super::bot_activity;
use super::state::{CoreState, Seen};
use super::{IMAGE, installed, listing, preview};

/// 带响应头的预设回应（如 `X-Wonton-Error`）：排在前头，先于不带头的查。
fn headered_response(core: &CoreState, method: &Method, path: &str) -> Option<Response> {
    let headered = core
        .canned_headered
        .lock()
        .unwrap()
        .remove(&(method.clone(), path.to_owned()))
        .or_else(|| {
            core.standing_headered
                .lock()
                .unwrap()
                .get(&(method.clone(), path.to_owned()))
                .cloned()
        })?;
    let Headered(status, headers, body) = headered;
    let mut builder = Response::builder().status(StatusCode::from_u16(status).unwrap());
    for (name, value) in headers {
        builder = builder.header(name, value);
    }
    Some(builder.body(Body::from(body)).unwrap())
}

/// 默认没有 Bot 钉页；单独要钉页的测试自己在前面拦。
fn empty_pins() -> Response {
    (
        StatusCode::OK,
        serde_json::json!({"pins": [], "consents": []}).to_string(),
    )
        .into_response()
}

/// 带延迟的预设回应：弹出就睡，保证这份旧回应一定是发得早的那个请求取走、晚于新回应到达。
async fn delayed_canned_response(
    core: &CoreState,
    method: &Method,
    path: &str,
) -> Option<Response> {
    let delayed = core
        .canned_delayed
        .lock()
        .unwrap()
        .get_mut(&(method.clone(), path.to_owned()))
        .and_then(|queue| queue.pop_front());
    let (Canned(status, body), delay) = delayed?;
    tokio::time::sleep(delay).await;
    Some(
        Response::builder()
            .status(StatusCode::from_u16(status).unwrap())
            .body(Body::from(body))
            .unwrap(),
    )
}

/// GET /_wonton/api/plugins：依次给出预设的列表，最后一份一直用下去。
fn installed_response(core: &CoreState) -> Response {
    let mut lists = core.lists.lock().unwrap();
    let plugins = if lists.len() > 1 {
        lists.pop_front().unwrap()
    } else {
        lists.front().cloned().unwrap_or_default()
    };
    (
        StatusCode::OK,
        serde_json::to_string(&InstalledPlugins {
            plugins,
            upgrade_check: None,
        })
        .unwrap(),
    )
        .into_response()
}

pub async fn fake_core(State(core): State<Arc<CoreState>>, request: Request) -> Response {
    let (parts, body) = request.into_parts();
    let path = parts
        .uri
        .path_and_query()
        .map_or(String::new(), ToString::to_string);
    let body = to_bytes(body, usize::MAX).await.unwrap();
    core.seen.lock().unwrap().push(Seen {
        method: parts.method.clone(),
        path: path.clone(),
        language: parts
            .headers
            .get(header::ACCEPT_LANGUAGE)
            .map(|v| v.to_str().unwrap().to_owned()),
        from_page: parts.headers.contains_key(header::ORIGIN)
            || parts.headers.contains_key("sec-fetch-site"),
        body: String::from_utf8_lossy(&body).into_owned(),
    });
    wait_for_release(&core, &parts.method, parts.uri.path()).await;
    if path == IMAGE {
        return (StatusCode::OK, vec![0x89, 0x50]).into_response();
    }
    if let Some(response) = headered_response(&core, &parts.method, parts.uri.path()) {
        return response;
    }
    if let Some(response) = delayed_canned_response(&core, &parts.method, parts.uri.path()).await {
        return response;
    }
    let canned = core
        .canned
        .lock()
        .unwrap()
        .get_mut(&(parts.method.clone(), parts.uri.path().to_owned()))
        .and_then(|queue| queue.pop_front())
        .or_else(|| {
            core.standing
                .lock()
                .unwrap()
                .get(&(parts.method.clone(), parts.uri.path().to_owned()))
                .cloned()
        });
    if let Some(Canned(status, body)) = canned {
        return Response::builder()
            .status(status)
            .body(Body::from(body))
            .unwrap();
    }
    if let Some(response) = bot_activity::respond(&core, &parts.method, &parts.uri).await {
        return response;
    }
    if let Some(response) =
        crate::family_support::respond(&core.family, &parts.method, parts.uri.path(), &body)
    {
        return response;
    }
    if let Some(response) = onboarding_routes(&core, &parts.method, parts.uri.path(), &body).await {
        return response;
    }
    let json = |value: String| (StatusCode::OK, value).into_response();
    match (&parts.method, parts.uri.path()) {
        (&Method::GET, "/_wonton/api/plugins") => installed_response(&core),
        (&Method::GET, "/_wonton/api/store") => json(serde_json::to_string(&listing()).unwrap()),
        (&Method::GET, "/_wonton/api/panel/pins") => empty_pins(),
        (&Method::POST, "/_wonton/api/plugins/preview") => {
            // 来源 github:owner/<id> 预览出来就是 <id>。
            let id = serde_json::from_slice::<serde_json::Value>(&body)
                .ok()
                .and_then(|body| body["source"].as_str().map(str::to_owned))
                .and_then(|source| source.strip_prefix("github:owner/").map(str::to_owned))
                .unwrap_or_else(|| "app.photos".to_owned());
            json(serde_json::to_string(&preview(&id)).unwrap())
        }
        (&Method::POST, "/_wonton/api/plugins") => json(
            serde_json::to_string(&installed(
                "app.photos",
                PluginState::Running {
                    health: PluginHealth::Healthy,
                },
            ))
            .unwrap(),
        ),
        (&Method::DELETE, _) => json("{}".to_owned()),
        _ => {
            let Canned(status, body) = core_error(404, "not_found", "没有这个");
            (StatusCode::from_u16(status).unwrap(), body).into_response()
        }
    }
}

async fn wait_for_release(core: &CoreState, method: &Method, path: &str) {
    let held = core
        .holds
        .lock()
        .unwrap()
        .get(&(method.clone(), path.to_owned()))
        .cloned();
    if let Some(held) = held {
        let mut released = held.subscribe();
        released.wait_for(|held| !*held).await.ok();
    }
}

async fn onboarding_routes(
    core: &Arc<CoreState>,
    method: &Method,
    path: &str,
    body: &[u8],
) -> Option<Response> {
    if path == "/_wonton/api/onboarding" {
        return Some(match *method {
            Method::GET => core
                .onboarding
                .lock()
                .unwrap()
                .clone()
                .map(json_value)
                .unwrap_or_else(|| core_error_response(404)),
            Method::PUT => {
                let mut state = core.onboarding.lock().unwrap();
                let Some(value) = state.as_mut() else {
                    return Some(core_error_response(404));
                };
                if let Ok(update) = serde_json::from_slice::<serde_json::Value>(body)
                    && let Some(next) = update.get("step").filter(|value| !value.is_null())
                {
                    value["step"] = next.clone();
                }
                json_value(value.clone())
            }
            _ => core_error_response(405),
        });
    }
    if path == wonton_protocol::ME_PATH {
        let mut owner = core.owner.lock().unwrap();
        let value = owner.get_or_insert_with(|| owner_json(None, None));
        if *method == Method::PUT
            && let Ok(update) = serde_json::from_slice::<serde_json::Value>(body)
        {
            value["display_name"] = update["display_name"].clone();
            value["email"] = update.get("email").cloned().unwrap_or_default();
        }
        return Some(json_value(value.clone()));
    }
    None
}

pub(super) fn owner_json(name: Option<&str>, email: Option<&str>) -> serde_json::Value {
    serde_json::json!({
        "user_id": "@owner:wonton.localhost",
        "username": "owner",
        "is_owner": true,
        "display_name": name,
        "email": email,
    })
}
