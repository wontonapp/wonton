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
    Gateway, TunnelPeer, api, bots, confirmations, core_upgrade, devices, family, files, friends,
    isolation, matrix, me, model_providers, onboarding, pages, peers, push, same_origin_path,
    settings_api, sso, store_api, tool_policy, with_cookie,
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
            &wonton_protocol::plugin_scope_path("{id}"),
            put(super::plugin_scope::scope),
        )
        .route(
            &wonton_protocol::plugin_settings_path("{id}"),
            get(super::plugin_settings::get).put(super::plugin_settings::set),
        )
        .route(
            &wonton_protocol::plugin_settings_preview_path("{id}"),
            post(super::plugin_settings::preview),
        )
        .route(
            "/_wonton/api/plugins/{id}/secrets",
            get(super::owner_secrets::list),
        )
        .route(
            "/_wonton/api/plugins/{id}/secrets/{name}",
            get(super::owner_secrets::value),
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
            put(settings_api::set_lan_webui),
        )
        .route(
            "/_wonton/api/plugins/{id}/limits",
            put(settings_api::set_limits),
        )
        .route(
            "/_wonton/api/plugins/{id}/permissions/{permission}",
            put(api::set_plugin_permission),
        )
        .route(
            "/_wonton/api/plugins/{id}/secrets/{name}",
            put(settings_api::set_secret_input),
        )
        .route(
            "/_wonton/api/plugins/{id}/ask-every-update",
            put(settings_api::set_ask_every_update),
        )
        .route(
            "/_wonton/api/plugins/{id}/save-grants",
            delete(settings_api::revoke_save_grant),
        )
        .route("/_wonton/api/store", get(store_api::store))
        .route("/_wonton/api/store/plugin", get(store_api::store_plugin))
        .route(
            "/_wonton/api/store/sources",
            post(store_api::add_store_source).delete(store_api::remove_store_source),
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

/// Owner-managed sharing; invitations remain standard Matrix requests (ADR-0051).
fn sharing_routes() -> Router<Arc<Gateway>> {
    use super::sharing::api;

    Router::new()
        .route("/_wonton/api/shares", post(api::create))
        .route(
            "/_wonton/api/shares/{id}",
            put(api::update).delete(api::revoke),
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
        .route(
            &wonton_protocol::bot_plugin_access_path("{id}", "{bot}", "{target}"),
            put(super::plugin_scope::bot_access),
        )
        .route(wonton_protocol::SHARED_BOTS_PATH, post(bots::create_shared))
        .merge(super::bot_contacts::routes())
        .route(
            "/_wonton/api/lan",
            get(settings_api::lan).put(settings_api::set_lan),
        )
        .route("/_wonton/api/storage", get(settings_api::storage))
        .route(
            "/_wonton/api/egress-proxy",
            get(settings_api::egress_proxy).put(settings_api::set_egress_proxy),
        )
        .route(
            "/_wonton/api/registry-mirror",
            get(super::registry_mirror::get).put(super::registry_mirror::put),
        )
        .route(
            "/_wonton/api/registry-mirror/test",
            post(super::registry_mirror::test),
        )
        .route(wonton_protocol::ME_PATH, get(me::read).put(me::save))
        .merge(link_preview_routes())
        .merge(member_routes())
        .merge(panel_routes())
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
        .merge(friend_routes())
        .merge(sharing_routes())
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
                pages::Format::of(&headers),
            )
        })
        .layer(middleware::from_fn_with_state(
            gateway.clone(),
            reject_cross_site_writes,
        ))
        .with_state(gateway)
}

/// 聊天页面板（ADR-0052）和 MCP Apps：Bot 活动、钉页，以及界面的宿主页和转发。
fn panel_routes() -> Router<Arc<Gateway>> {
    Router::new()
        .route(
            wonton_protocol::BOT_ACTIVITY_PATH,
            get(super::bot_activity::get),
        )
        .route(
            wonton_protocol::BOT_PANEL_PINS_PATH,
            get(super::panel_pins::list),
        )
        .route(
            &wonton_protocol::bot_panel_pin_path("{id}"),
            delete(super::panel_pins::remove),
        )
        .route(
            &wonton_protocol::bot_panel_pin_seen_path("{id}"),
            post(super::panel_pins::seen),
        )
        .route(
            &wonton_protocol::bot_panel_consent_path("{plugin_id}", "{bot}"),
            put(super::panel_pins::consent),
        )
        .route(
            &wonton_protocol::mcp_app_host_path("{pin}"),
            get(super::mcp_apps::host),
        )
        .route(
            wonton_protocol::MCP_APP_CALLS_PATH,
            post(super::mcp_apps::call),
        )
        .route(
            &wonton_protocol::mcp_app_call_path("{id}"),
            get(super::mcp_apps::result),
        )
        .route(
            wonton_protocol::MCP_APP_READ_PATH,
            post(super::mcp_apps::read),
        )
        .route(
            wonton_protocol::MCP_APP_TOOLS_PATH,
            post(super::mcp_apps::tools),
        )
        .route(
            wonton_protocol::MCP_APP_MESSAGE_PATH,
            post(super::mcp_apps::message),
        )
}

/// 链接预览（ADR-0065）：核心去取网页摘要和预览图。
fn link_preview_routes() -> Router<Arc<Gateway>> {
    Router::new()
        .route(
            wonton_protocol::LINK_PREVIEW_PATH,
            get(super::link_preview::preview),
        )
        .route(
            &wonton_protocol::link_preview_image_path("{id}"),
            get(super::link_preview::image),
        )
}

/// 好友名单和联邦黑名单（ADR-0051）。个人屏蔽走 Matrix 账号数据，不在这里。
fn friend_routes() -> Router<Arc<Gateway>> {
    Router::new()
        .route("/_wonton/api/friends", get(friends::list))
        .route(
            "/_wonton/api/friends/{user_id}",
            put(friends::accept).delete(friends::remove),
        )
        .route(
            "/_wonton/api/friends/{user_id}/groups",
            put(friends::set_groups),
        )
        .route("/_wonton/api/friend-groups", post(friends::save_group))
        .route(
            "/_wonton/api/friend-groups/{id}",
            delete(friends::remove_group),
        )
        .route(
            "/_wonton/api/federation-blocks/{server_name}",
            post(friends::block_server).delete(friends::unblock_server),
        )
}

/// 家人和共享 Bot（ADR-0045）：邀请、设置、配对码、移除、插件开放。
fn member_routes() -> Router<Arc<Gateway>> {
    Router::new()
        .route(
            wonton_protocol::MEMBERS_PATH,
            get(family::list).post(family::invite),
        )
        .route("/_wonton/api/members/{username}", put(family::settings))
        .route(
            "/_wonton/api/members/{username}/pairing-code",
            post(family::pairing_code).delete(family::void_pairing_code),
        )
        .route(
            "/_wonton/api/members/{username}/remove",
            post(family::remove),
        )
        .route(
            "/_wonton/api/plugins/{id}/family",
            put(family::set_plugin_family),
        )
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
        pages::Format::of(req.headers()),
    )
}

async fn home(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
) -> Response {
    let langs = api::languages(&headers);
    match gateway.core_person(peer.map(|Extension(peer)| peer), &headers) {
        Some(person) => pages::status(&person.username, &gateway.plugins.list(), &langs),
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
    // 主人令牌登录的核心会话是主人（ADR-0045）。
    let person = super::Person::owner(gateway.owner.username());
    let sid = gateway.sessions.create_core(&person);
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
