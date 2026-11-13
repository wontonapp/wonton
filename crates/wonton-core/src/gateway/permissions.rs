//! 权限项守卫（ADR-0088）：每个管理接口写明自己要哪一项，主人一律放行，其余按当前角色的权限项判断。
//! 被挡下时 403，`PERMISSION_REQUIRED` 头和 [`PermissionRequired`] 体说明缺的是哪一项。
//!
//! 判断在每个请求上重新做：角色改了，下一个请求就按新角色算；主人令牌和主人名下的设备都不进角色。

use axum::Extension;
use axum::Json;
use axum::response::{IntoResponse, Response};
use hyper::header::HeaderValue;
use hyper::{HeaderMap, StatusCode};
use wonton_protocol::{PERMISSION_REQUIRED, PermissionItem, PermissionRequired};

use super::api::{denied, from_another_page};
use super::{ERROR_HEADER, Gateway, Person, TunnelPeer};

/// 请求要过 `item` 这一项。主人正在用的令牌直接放行；设备身份先认人，再按他的角色判断。
pub(crate) fn require(
    gateway: &Gateway,
    peer: Option<Extension<TunnelPeer>>,
    headers: &HeaderMap,
    item: PermissionItem,
) -> Option<Response> {
    denied(gateway, headers)?;
    let person = match gateway.device_person(peer.map(|Extension(peer)| peer)) {
        Some(person) => person,
        None => return denied(gateway, headers),
    };
    if let Some(cross_site) = from_another_page(headers, &gateway.core_origin()) {
        return Some(cross_site);
    }
    missing(gateway, &person, item)
}

/// 已经认出人的接口用这个：主人放行，其余按角色判断。
pub(crate) fn require_person(
    gateway: &Gateway,
    person: &Person,
    item: PermissionItem,
) -> Option<Response> {
    missing(gateway, person, item)
}

fn missing(gateway: &Gateway, person: &Person, item: PermissionItem) -> Option<Response> {
    if person.is_owner() || gateway.permissions(person).allows(item) {
        return None;
    }
    let mut response = (
        StatusCode::FORBIDDEN,
        Json(PermissionRequired { missing: item }),
    )
        .into_response();
    response
        .headers_mut()
        .insert(ERROR_HEADER, HeaderValue::from_static(PERMISSION_REQUIRED));
    Some(response)
}
