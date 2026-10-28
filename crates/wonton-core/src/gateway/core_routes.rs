//! 核心域名 `<base>` 上的路由。

use std::sync::Arc;

use axum::extract::{Form, Query, State};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use hyper::HeaderMap;
use serde::Deserialize;

use super::session::{CORE_COOKIE, CORE_TTL, cookie, set_cookie};
use super::{Gateway, api, pages, same_origin_path, sso, with_cookie};

pub fn routes(gateway: Arc<Gateway>) -> Router {
    Router::new()
        .route("/", get(home))
        .route(
            "/_wonton/health",
            get(|| async { Json(serde_json::json!({"status": "ok"})) }),
        )
        .route("/_wonton/login", get(login_page).post(login))
        .route("/_wonton/logout", post(logout))
        .route("/_wonton/sso", get(sso::start))
        .route("/_wonton/oidc/login", get(sso::oidc_login))
        .route("/_wonton/oidc/consent", get(sso::oidc_consent))
        .route("/_wonton/api/plugins", get(api::list).post(api::install))
        .route("/_wonton/api/plugins/preview", post(api::preview))
        .route("/_wonton/api/plugins/{id}", delete(api::uninstall))
        .fallback(|| async {
            pages::error(hyper::StatusCode::NOT_FOUND, "not_found", "没有这个页面")
        })
        .with_state(gateway)
}

async fn home(State(gateway): State<Arc<Gateway>>, headers: HeaderMap) -> Response {
    match gateway.core_user(&headers) {
        Some(user) => pages::status(&user, &gateway.plugins.list()),
        None => pages::login("/", false),
    }
}

#[derive(Deserialize)]
struct NextQuery {
    #[serde(default)]
    next: Option<String>,
}

async fn login_page(Query(query): Query<NextQuery>) -> Response {
    pages::login(query.next.as_deref().unwrap_or("/"), false)
}

#[derive(Deserialize)]
struct LoginForm {
    token: String,
    #[serde(default)]
    next: Option<String>,
}

async fn login(State(gateway): State<Arc<Gateway>>, Form(form): Form<LoginForm>) -> Response {
    let next = safe_next(&gateway, form.next.as_deref());
    if !gateway.check_owner_token(&form.token) {
        tracing::warn!("主人令牌不正确");
        return pages::login(&next, true);
    }
    let sid = gateway.sessions.create_core(&gateway.owner.id);
    let cookie = set_cookie(CORE_COOKIE, &sid, CORE_TTL, gateway.secure_cookies());
    with_cookie(Redirect::to(&next).into_response(), cookie)
}

async fn logout(State(gateway): State<Arc<Gateway>>, headers: HeaderMap) -> Response {
    if let Some(sid) = cookie(&headers, CORE_COOKIE) {
        gateway.sessions.revoke_core(&sid);
    }
    let expired = format!("{CORE_COOKIE}=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0");
    with_cookie(Redirect::to("/").into_response(), expired)
}

/// 登录后的跳转只能留在核心自己的页面上（站内相对路径或核心域名下的绝对地址）。
fn safe_next(gateway: &Gateway, next: Option<&str>) -> String {
    next.and_then(|next| same_origin_path(&gateway.core_origin(), next))
        .unwrap_or_else(|| "/".to_owned())
}
