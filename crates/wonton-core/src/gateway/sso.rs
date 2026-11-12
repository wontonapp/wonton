//! 单点登录：插件会话的交接，以及身份提供方回调核心的登录、授权确认端点。

use std::sync::Arc;

use axum::Extension;
use axum::extract::{Query, Request, State};
use axum::response::{IntoResponse, Redirect, Response};
use hyper::{HeaderMap, StatusCode};
use serde::Deserialize;

use super::session::{APP_COOKIE, APP_TTL, set_cookie};
use super::{Gateway, TunnelPeer, api, pages, same_origin_path, with_cookie};
use crate::config::HostKind;
use crate::l10n;
use crate::oidc::{OidcError, granted_scopes, id_token_claims};
use crate::plugin::languages::Languages;

#[derive(Deserialize)]
pub struct SsoQuery {
    to: String,
}

/// 未登录时跳到登录页，登录后回到当前地址。
fn login_redirect(gateway: &Gateway, path_and_query: &str) -> Response {
    let next = format!("{}{path_and_query}", gateway.core_origin());
    let query = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("next", &next)
        .finish();
    Redirect::to(&format!("/_wonton/login?{query}")).into_response()
}

fn path_and_query(req_uri: &hyper::Uri) -> String {
    req_uri
        .path_and_query()
        .map_or("/", |p| p.as_str())
        .to_owned()
}

/// `GET <base>/_wonton/sso?to=<插件地址>`：已登录则签发交接码，跳回插件域名兑换。
pub async fn start(
    State(gateway): State<Arc<Gateway>>,
    Query(query): Query<SsoQuery>,
    req: Request,
) -> Response {
    let Some(user) = gateway.core_user(req.extensions().get().copied(), req.headers()) else {
        return login_redirect(&gateway, &path_and_query(req.uri()));
    };
    let langs = api::languages(req.headers());
    let Ok(target) = url::Url::parse(&query.to) else {
        return pages::error(
            StatusCode::BAD_REQUEST,
            "invalid_input",
            &l10n::text(&langs, "gateway-sso-invalid-next", &[]),
            &langs,
        );
    };
    let host = target.host_str().unwrap_or_default().to_owned();
    let is_plugin = matches!(gateway.public.classify(&host), HostKind::Plugin(_));
    let origin_ok = target.scheme() == gateway.public.scheme
        && target.port_or_known_default() == Some(gateway.public.port);
    if !is_plugin || !origin_ok {
        return pages::error(
            StatusCode::BAD_REQUEST,
            "invalid_input",
            &l10n::text(&langs, "gateway-sso-local-only", &[]),
            &langs,
        );
    }
    let code = gateway.sessions.create_code(&user, &host);
    let mut path = target.path().to_owned();
    if let Some(q) = target.query() {
        path.push('?');
        path.push_str(q);
    }
    let params = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("code", &code)
        .append_pair("to", &path)
        .finish();
    let origin = gateway.public.origin(&host);
    Redirect::to(&format!("{origin}/_wonton/session?{params}")).into_response()
}

#[derive(Deserialize)]
struct RedeemQuery {
    code: String,
    to: String,
}

/// `GET <slug>.<base>/_wonton/session?code=..&to=..`：兑换交接码，设置本域名的插件会话。
pub fn redeem(gateway: &Gateway, host: &str, req: &Request) -> Response {
    let langs = api::languages(req.headers());
    let Ok(Query(query)) = Query::<RedeemQuery>::try_from_uri(req.uri()) else {
        return pages::error(
            StatusCode::BAD_REQUEST,
            "invalid_input",
            &l10n::text(&langs, "gateway-sso-missing-code", &[]),
            &langs,
        );
    };
    let Some(user) = gateway.sessions.redeem_code(&query.code, host) else {
        return pages::error(
            StatusCode::UNAUTHORIZED,
            "unauthenticated",
            &l10n::text(&langs, "gateway-sso-code-invalid", &[]),
            &langs,
        );
    };
    // 只允许跳到本域名内的路径，防止开放重定向。
    let to =
        same_origin_path(&gateway.public.origin(host), &query.to).unwrap_or_else(|| "/".to_owned());
    let sid = gateway.sessions.create_app(&user, host);
    let cookie = set_cookie(APP_COOKIE, &sid, APP_TTL, gateway.secure_cookies());
    with_cookie(Redirect::to(&to).into_response(), cookie)
}

#[derive(Deserialize)]
pub struct LoginChallenge {
    login_challenge: String,
}

#[derive(Deserialize)]
pub struct ConsentChallenge {
    consent_challenge: String,
}

fn oidc_failure(e: &OidcError, langs: &Languages) -> Response {
    let (status, kind) = match e {
        OidcError::Unavailable(_) => (StatusCode::BAD_GATEWAY, "oidc_unavailable"),
        OidcError::Rejected {
            status: 404 | 410, ..
        } => (StatusCode::BAD_REQUEST, "invalid_input"),
        _ => (StatusCode::BAD_GATEWAY, "oidc_error"),
    };
    tracing::warn!(error = %e, "OIDC 流程失败");
    pages::error(status, kind, &e.to_string(), langs)
}

/// Hydra 的登录端点：用核心会话或隧道设备的身份直接接受登录，用户不会看到第二个登录页。
pub async fn oidc_login(
    State(gateway): State<Arc<Gateway>>,
    Query(query): Query<LoginChallenge>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
    uri: hyper::Uri,
) -> Response {
    let langs = api::languages(&headers);
    let Some(user) = gateway.core_user(peer.map(|Extension(peer)| peer), &headers) else {
        return login_redirect(&gateway, &path_and_query(&uri));
    };
    let Some(hydra) = &gateway.hydra else {
        return pages::error(
            StatusCode::SERVICE_UNAVAILABLE,
            "oidc_unavailable",
            &l10n::text(&langs, "gateway-oidc-unconfigured", &[]),
            &langs,
        );
    };
    let request = match hydra.login_request(&query.login_challenge).await {
        Ok(request) => request,
        Err(e) => return oidc_failure(&e, &langs),
    };
    if request.skip && request.subject != user {
        return pages::error(
            StatusCode::CONFLICT,
            "conflict",
            &l10n::text(&langs, "gateway-sso-user-mismatch", &[]),
            &langs,
        );
    }
    tracing::info!(client = %request.client.client_id, %user, "接受 OIDC 登录");
    match hydra.accept_login(&query.login_challenge, &user).await {
        Ok(to) => Redirect::to(&to).into_response(),
        Err(e) => oidc_failure(&e, &langs),
    }
}

/// Hydra 的授权确认端点：插件由主人安装，即视为已授权；只发放清单内的 scope 和对应的声明。
pub async fn oidc_consent(
    State(gateway): State<Arc<Gateway>>,
    Query(query): Query<ConsentChallenge>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
    uri: hyper::Uri,
) -> Response {
    let langs = api::languages(&headers);
    let Some(user) = gateway.core_user(peer.map(|Extension(peer)| peer), &headers) else {
        return login_redirect(&gateway, &path_and_query(&uri));
    };
    let Some(hydra) = &gateway.hydra else {
        return pages::error(
            StatusCode::SERVICE_UNAVAILABLE,
            "oidc_unavailable",
            &l10n::text(&langs, "gateway-oidc-unconfigured", &[]),
            &langs,
        );
    };
    let request = match hydra.consent_request(&query.consent_challenge).await {
        Ok(request) => request,
        Err(e) => return oidc_failure(&e, &langs),
    };
    if request.subject != user || user != gateway.owner.username() {
        return pages::error(
            StatusCode::FORBIDDEN,
            "permission_denied",
            &l10n::text(&langs, "gateway-sso-consent-mismatch", &[]),
            &langs,
        );
    }
    let granted = granted_scopes(&request.requested_scope);
    let groups = gateway.plugins.oidc_owner_groups(&request.client.client_id);
    let claims = id_token_claims(&gateway.owner.owner(), &granted, &groups);
    tracing::info!(client = %request.client.client_id, ?granted, ?groups, "接受 OIDC 授权");
    match hydra
        .accept_consent(&query.consent_challenge, &granted, claims)
        .await
    {
        Ok(to) => Redirect::to(&to).into_response(),
        Err(e) => oidc_failure(&e, &langs),
    }
}
