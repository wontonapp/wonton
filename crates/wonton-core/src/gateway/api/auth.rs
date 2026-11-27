//! 插件管理接口的鉴权：接口只接受主人令牌或经隧道的已配对设备，设备的请求还要不是别的网页发起的
//! （ADR-0017、ADR-0045、ADR-0088）。取到的人交给处理器按人过滤。

use axum::Extension;
use axum::response::Response;
use hyper::header::AUTHORIZATION;
use hyper::{HeaderMap, StatusCode};

use super::{
    Gateway, Person, TunnelPeer, error_response, install_error, install_gates, isolation,
    languages, permissions,
};
use crate::l10n;
use crate::plugin::InstallError;
use wonton_protocol::PermissionItem;

/// 带没带主人令牌。
pub(crate) fn has_owner_token(gateway: &Gateway, headers: &HeaderMap) -> bool {
    headers
        .get(AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .is_some_and(|token| gateway.check_owner_token(token))
}

/// 只接受 `Authorization: Bearer <主人令牌>`：浏览器 Cookie 随请求自动带上，插件网页可以借此发起跨站请求。未授权时
/// 返回要直接回给客户端的响应。
pub(crate) fn denied(gateway: &Gateway, headers: &HeaderMap) -> Option<Response> {
    let token = headers
        .get(AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "));
    match token {
        Some(token) if gateway.check_owner_token(token) => None,
        _ => Some(error_response(
            StatusCode::UNAUTHORIZED,
            "unauthenticated",
            &l10n::text(&languages(headers), "gateway-denied", &[]),
        )),
    }
}

/// 插件管理的读接口和每个人自己的接口：主人令牌，或者任何已配对设备（经隧道，且不是别的网页发起的）。
/// 设备是谁由调用方按人处理：家人的插件列表只列开放了的，设备列表只有自己的。
pub(crate) fn plugin_denied(
    gateway: &Gateway,
    peer: Option<Extension<TunnelPeer>>,
    headers: &HeaderMap,
) -> Option<Response> {
    denied(gateway, headers)?;
    match gateway.device_person(peer.map(|Extension(peer)| peer)) {
        Some(_) => from_another_page(headers, &gateway.core_origin()),
        None => denied(gateway, headers),
    }
}

/// 设备的请求还要不是别的网页发起的（ADR-0017）：隧道身份随请求自动带上，App 的 WebView 里打开的插件网页
/// 发的请求也带着它，插件网页不能借它改管理设置。是别的网页发起时给拒绝。
pub(crate) fn from_another_page(headers: &HeaderMap, origin: &str) -> Option<Response> {
    if isolation::not_from_another_page(headers, origin) {
        return None;
    }
    Some(error_response(
        StatusCode::FORBIDDEN,
        "cross_site",
        &l10n::text(&languages(headers), "gateway-cross-site-api", &[]),
    ))
}

/// 过了 [`plugin_denied`] 的请求是谁发的，写审计用：主人令牌是 `owner_token`，设备是 `<用户名>/device:<id>`。
pub(crate) fn acted_by(
    gateway: &Gateway,
    peer: Option<&Extension<TunnelPeer>>,
    headers: &HeaderMap,
) -> String {
    if denied(gateway, headers).is_none() {
        return "owner_token".to_owned();
    }
    if let Some(Extension(TunnelPeer(device))) = peer.copied()
        && let Some(person) = gateway.device_person(Some(TunnelPeer(device)))
    {
        return format!("{}/device:{device}", person.username);
    }
    "owner_token".to_owned()
}

/// 这些接口不接受会话 Cookie：请求的人要么带着主人令牌（主人），要么是经隧道的已配对设备（设备记录里的人）。
/// [`plugin_denied`] 已经把两样都没有的请求挡掉了。鉴权和取人之间设备记录可能消失（家人刚被移除），
/// 这时按认不出处理，回 [`plugin_denied`] 同样的拒绝，不编一个人出来。
pub(crate) fn requester(
    gateway: &Gateway,
    peer: Option<Extension<TunnelPeer>>,
    headers: &HeaderMap,
) -> Result<Person, Box<Response>> {
    if denied(gateway, headers).is_none() {
        gateway.note_languages(None, headers);
        return Ok(Person::owner(gateway.owner.username()));
    }
    let person = gateway
        .device_person(peer.map(|Extension(peer)| peer))
        .ok_or_else(|| {
            Box::new(error_response(
                StatusCode::UNAUTHORIZED,
                "unauthenticated",
                &l10n::text(&languages(headers), "gateway-denied", &[]),
            ))
        })?;
    gateway.note_languages(peer.map(|Extension(peer)| peer), headers);
    Ok(person)
}

pub(crate) fn current_requester(
    gateway: &Gateway,
    peer: Option<Extension<TunnelPeer>>,
    headers: &HeaderMap,
    expected: &Person,
) -> Result<Person, Box<Response>> {
    let current = requester(gateway, peer, headers)?;
    if current != *expected {
        return Err(Box::new(error_response(
            StatusCode::UNAUTHORIZED,
            "unauthenticated",
            &l10n::text(&languages(headers), "gateway-denied", &[]),
        )));
    }
    Ok(current)
}

pub(crate) fn current_permitted_requester(
    gateway: &Gateway,
    peer: Option<Extension<TunnelPeer>>,
    headers: &HeaderMap,
    expected: &Person,
    permission: PermissionItem,
) -> Result<Person, Box<Response>> {
    let person = current_requester(gateway, peer, headers, expected)?;
    if let Some(denied) = permissions::require_person(gateway, &person, permission) {
        return Err(Box::new(denied));
    }
    Ok(person)
}

pub(crate) fn response_for_requester<T>(
    gateway: &Gateway,
    peer: Option<Extension<TunnelPeer>>,
    headers: &HeaderMap,
    expected: &Person,
    result: Result<T, InstallError>,
    render: impl FnOnce(&Gateway, Option<TunnelPeer>, &Person, T) -> Response,
) -> Response {
    let person = match current_requester(gateway, peer, headers, expected) {
        Ok(person) => person,
        Err(denied) => return *denied,
    };
    if let Some(denied) = install_gates::deny_check(gateway, &person) {
        return denied;
    }
    match result {
        Ok(value) => render(gateway, peer.map(|Extension(peer)| peer), &person, value),
        Err(error) => install_error(&error),
    }
}
