//! 核心域名 `<base>` 上的路由。改状态的请求只接受同源（`isolation::write_allowed`）。

use std::sync::Arc;

use axum::Extension;
use axum::extract::{DefaultBodyLimit, Form, Query, Request, State};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::{delete, get, post, put};
use axum::{Json, Router};
use hyper::{HeaderMap, StatusCode};
use serde::Deserialize;

use crate::l10n;

use super::session::{CORE_COOKIE, CORE_TTL, cookies, set_cookie};
use super::{
    Gateway, TunnelPeer, api, confirmations, core_upgrade, devices, files, isolation, matrix,
    model_providers, onboarding, owner_profile, pages, peers, push, same_origin_path, sso,
    tool_policy, with_cookie,
};

/// 确认卡片，和主人对确认的放宽（ADR-0016、ADR-0024）。
fn confirmation_routes() -> Router<Arc<Gateway>> {
    Router::new()
        .route("/_wonton/confirmations/{id}", post(confirmations::answer))
        .route(
            "/_wonton/confirmations/{id}/window",
            delete(confirmations::stop_window),
        )
        .route("/_wonton/api/confirmations", get(confirmations::pending))
        .route(
            "/_wonton/api/plugins/{id}/tool-policy",
            get(tool_policy::view),
        )
        .route(
            "/_wonton/api/plugins/{id}/tool-policy/always/{caller}",
            put(tool_policy::always).delete(tool_policy::always),
        )
        .route(
            "/_wonton/api/plugins/{id}/tool-policy/allow-all/{caller}",
            put(tool_policy::allow_all).delete(tool_policy::allow_all),
        )
        .route(
            "/_wonton/api/plugins/{id}/tool-policy/read-only/{tool}",
            put(tool_policy::read_only).delete(tool_policy::read_only),
        )
        .route(
            "/_wonton/api/plugins/{id}/takeover",
            put(tool_policy::takeover).delete(tool_policy::takeover),
        )
}

/// 密钥请求显式限 8 KiB，鉴权和同源限制与插件管理接口相同。
fn model_provider_routes() -> Router<Arc<Gateway>> {
    Router::new()
        .route(
            wonton_protocol::MODEL_PROVIDERS_PATH,
            get(model_providers::list),
        )
        .route(
            "/_wonton/api/models/providers/{id}",
            put(model_providers::save),
        )
        .route(
            "/_wonton/api/models/providers/{id}/test",
            post(model_providers::test),
        )
        .route(
            wonton_protocol::CUSTOM_MODEL_PROVIDER_PATH,
            put(model_providers::custom_save),
        )
        .route(
            wonton_protocol::CUSTOM_MODEL_PROVIDER_TEST_PATH,
            post(model_providers::custom_test),
        )
        .layer(DefaultBodyLimit::max(8 << 10))
}

/// 插件管理这一组路由：装、看、升级、开关，都挂在 `/_wonton/api/plugins` 下。
fn plugin_routes() -> Router<Arc<Gateway>> {
    Router::new()
        .route(
            "/_wonton/api/plugins",
            get(api::list)
                .post(api::install)
                .layer(DefaultBodyLimit::max(api::MAX_UPLOAD_BYTES)),
        )
        .route(
            "/_wonton/api/plugins/preview",
            post(api::preview).layer(DefaultBodyLimit::max(api::MAX_UPLOAD_BYTES)),
        )
        .route(
            "/_wonton/api/plugins/{id}",
            get(api::detail).delete(api::uninstall),
        )
        .route(
            "/_wonton/api/plugins/{id}/start",
            post(super::plugin_start::start),
        )
        .route(
            "/_wonton/api/plugins/{id}/upgrade",
            post(api::upgrade).layer(DefaultBodyLimit::max(api::MAX_UPLOAD_BYTES)),
        )
        .route(
            "/_wonton/api/plugins/{id}/upgrade/preview",
            post(api::preview_upgrade).layer(DefaultBodyLimit::max(api::MAX_UPLOAD_BYTES)),
        )
        .route(
            "/_wonton/api/plugins/{id}/lan-webui",
            put(api::set_lan_webui),
        )
        .route("/_wonton/api/plugins/{id}/limits", put(api::set_limits))
        .route(
            "/_wonton/api/plugins/{id}/ask-every-update",
            put(api::set_ask_every_update),
        )
        .route(
            "/_wonton/api/plugins/{id}/save-grants",
            delete(api::revoke_save_grant),
        )
        .route("/_wonton/api/store", get(api::store))
        .route("/_wonton/api/store/plugin", get(api::store_plugin))
        .route(
            "/_wonton/api/store/sources",
            post(api::add_store_source).delete(api::remove_store_source),
        )
        .route("/_wonton/api/images/{sha256}/{file}", get(api::image))
}

/// 服务器配对的管理接口（ADR-0050）。
fn peer_routes() -> Router<Arc<Gateway>> {
    Router::new()
        .route(
            wonton_protocol::PEERS_PATH,
            get(peers::list).post(peers::scan),
        )
        .route(&wonton_protocol::peer_code_path(), get(peers::code))
        .route(
            &wonton_protocol::peer_code_reset_path(),
            post(peers::reset_code),
        )
        .route(
            &wonton_protocol::peer_request_approve_path("{id}"),
            post(peers::approve),
        )
        .route(
            &wonton_protocol::peer_request_reject_path("{id}"),
            post(peers::reject),
        )
        .route(
            &wonton_protocol::peer_rejected_path("{id}"),
            delete(peers::unreject),
        )
        .route(
            &wonton_protocol::peer_path("{server_name}"),
            delete(peers::remove),
        )
}

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
        .route("/_wonton/matrix/login-token", post(matrix::login_token))
        .merge(model_provider_routes())
        .route(
            "/_wonton/push/registration",
            put(push::register).delete(push::unregister),
        )
        .merge(plugin_routes())
        .route(
            &wonton_protocol::bot_profile_path("{id}"),
            put(super::bot_profile::save),
        )
        .route("/_wonton/api/lan", get(api::lan).put(api::set_lan))
        .route("/_wonton/api/storage", get(api::storage))
        .route(
            "/_wonton/api/egress-proxy",
            get(api::egress_proxy).put(api::set_egress_proxy),
        )
        .route(
            "/_wonton/api/registry-mirror",
            get(super::registry_mirror::get).put(super::registry_mirror::put),
        )
        .route(
            "/_wonton/api/registry-mirror/test",
            post(super::registry_mirror::test),
        )
        .route(
            wonton_protocol::OWNER_PROFILE_PATH,
            get(owner_profile::read).put(owner_profile::save),
        )
        .route(
            wonton_protocol::ONBOARDING_PATH,
            get(onboarding::get).put(onboarding::put),
        )
        .route(
            wonton_protocol::CORE_UPGRADE_PATH,
            get(core_upgrade::status)
                .post(core_upgrade::start)
                .delete(core_upgrade::cancel),
        )
        .route(
            wonton_protocol::CORE_UPGRADE_CHECK_PATH,
            post(core_upgrade::check),
        )
        .route("/_wonton/api/tunnel", get(devices::tunnel))
        .merge(peer_routes())
        .route(
            "/_wonton/api/devices",
            get(devices::list).post(devices::add),
        )
        .route("/_wonton/api/devices/{id}", delete(devices::remove))
        .route("/_wonton/api/pairing-codes", post(devices::pairing_code))
        .route(
            "/_wonton/api/pairing-codes/first",
            post(devices::first_pairing_code),
        )
        .merge(confirmation_routes())
        .merge(files::routes(gateway.clone()))
        .fallback(|headers: HeaderMap| async move {
            let langs = api::languages(&headers);
            pages::error(
                StatusCode::NOT_FOUND,
                "not_found",
                &l10n::text(&langs, "gateway-not-found", &[]),
                &langs,
            )
        })
        .layer(middleware::from_fn_with_state(
            gateway.clone(),
            reject_cross_site_writes,
        ))
        .with_state(gateway)
}

/// 插件网页和核心同站，能让浏览器带着核心会话向核心发表单或 `fetch`（比如替主人退出登录）。所以改状态的
/// 请求只接受核心自己的页面发起的。
async fn reject_cross_site_writes(
    State(gateway): State<Arc<Gateway>>,
    req: Request,
    next: Next,
) -> Response {
    if isolation::write_allowed(req.method(), req.headers(), &gateway.core_origin()) {
        return next.run(req).await;
    }
    tracing::warn!(method = %req.method(), path = req.uri().path(), "拒绝核心域名上的跨站修改请求");
    let langs = api::languages(req.headers());
    pages::error(
        StatusCode::FORBIDDEN,
        "cross_site",
        &l10n::text(&langs, "gateway-cross-site-write", &[]),
        &langs,
    )
}

async fn home(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
) -> Response {
    let langs = api::languages(&headers);
    match gateway.core_user(peer.map(|Extension(peer)| peer), &headers) {
        Some(user) => pages::status(&user, &gateway.plugins.list(), &langs),
        None => pages::login("/", false, &langs),
    }
}

#[derive(Deserialize)]
struct NextQuery {
    #[serde(default)]
    next: Option<String>,
}

async fn login_page(Query(query): Query<NextQuery>, headers: HeaderMap) -> Response {
    pages::login(
        query.next.as_deref().unwrap_or("/"),
        false,
        &api::languages(&headers),
    )
}

#[derive(Deserialize)]
struct LoginForm {
    token: String,
    #[serde(default)]
    next: Option<String>,
}

async fn login(
    State(gateway): State<Arc<Gateway>>,
    headers: HeaderMap,
    Form(form): Form<LoginForm>,
) -> Response {
    let next = safe_next(&gateway, form.next.as_deref());
    if !gateway.check_owner_token(&form.token) {
        tracing::warn!("主人令牌不正确");
        return pages::login(&next, true, &api::languages(&headers));
    }
    let sid = gateway.sessions.create_core(gateway.owner.username());
    let cookie = set_cookie(CORE_COOKIE, &sid, CORE_TTL, gateway.secure_cookies());
    with_cookie(Redirect::to(&next).into_response(), cookie)
}

async fn logout(State(gateway): State<Arc<Gateway>>, headers: HeaderMap) -> Response {
    for sid in cookies(&headers, CORE_COOKIE) {
        gateway.sessions.revoke_core(sid);
    }
    let expired = format!("{CORE_COOKIE}=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0");
    with_cookie(Redirect::to("/").into_response(), expired)
}

/// 登录后的跳转只能留在核心自己的页面上（站内相对路径或核心域名下的绝对地址）。
fn safe_next(gateway: &Gateway, next: Option<&str>) -> String {
    next.and_then(|next| same_origin_path(&gateway.core_origin(), next))
        .unwrap_or_else(|| "/".to_owned())
}
