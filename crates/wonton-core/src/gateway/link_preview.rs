//! 链接预览的网关接口（ADR-0065）：主人令牌和已配对设备（主人和家人的）能调，认人同插件管理的读接口
//! （[`plugin_denied`]）。Bot 的 Matrix 令牌、插件容器不带凭据的请求、App 里打开的插件网页借设备身份发的请求都被拒：
//! 不然它们就多了一条经核心出网的路。

use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use hyper::header::{self, HeaderValue};
use hyper::{HeaderMap, StatusCode};
use serde::Deserialize;
use wonton_protocol::LinkPreviewFailure;

use super::api::{error_response, languages, plugin_denied};
use super::{Gateway, TunnelPeer};
use crate::l10n;
use crate::link_preview::Failure;

/// 主人令牌的请求没有设备，限速时合成一个键。
const OWNER_TOKEN_KEY: &str = "owner_token";

#[derive(Deserialize)]
pub struct PreviewQuery {
    #[serde(default)]
    url: Option<String>,
}

/// `GET LINK_PREVIEW_PATH?url=`：成功回 [`wonton_protocol::LinkPreview`]，失败的 `kind` 是 [`LinkPreviewFailure`]。
pub async fn preview(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
    Query(query): Query<PreviewQuery>,
) -> Response {
    if let Some(denied) = plugin_denied(&gateway, peer, &headers) {
        return denied;
    }
    let device = peer.map_or_else(
        || OWNER_TOKEN_KEY.to_owned(),
        |Extension(TunnelPeer(id))| id.to_string(),
    );
    let url = query.url.unwrap_or_default();
    match gateway.link_previews.preview(&url, &device).await {
        Ok(preview) => Json(preview).into_response(),
        Err(failure) => failure_response(&failure, &headers),
    }
}

/// `GET link_preview_image_path(id)`：缓存里的预览图片，过期或没有时 404。
pub async fn image(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    if let Some(denied) = plugin_denied(&gateway, peer, &headers) {
        return denied;
    }
    let Some(image) = gateway.link_previews.image(&id) else {
        let langs = languages(&headers);
        return error_response(
            StatusCode::NOT_FOUND,
            "not_found",
            &l10n::text(&langs, "link-preview-image-gone", &[]),
        );
    };
    let mime = HeaderValue::from_str(&image.mime_type)
        .unwrap_or_else(|_| HeaderValue::from_static("application/octet-stream"));
    (
        [
            (header::CONTENT_TYPE, mime),
            (
                header::X_CONTENT_TYPE_OPTIONS,
                HeaderValue::from_static("nosniff"),
            ),
            (
                header::CACHE_CONTROL,
                HeaderValue::from_static("private, max-age=86400"),
            ),
        ],
        image.bytes,
    )
        .into_response()
}

fn failure_response(failure: &Failure, headers: &HeaderMap) -> Response {
    let langs = languages(headers);
    let (status, kind, message) = match failure.kind {
        LinkPreviewFailure::InvalidUrl => (
            StatusCode::BAD_REQUEST,
            "invalid_url",
            "link-preview-invalid-url",
        ),
        LinkPreviewFailure::BlockedAddress => (
            StatusCode::UNPROCESSABLE_ENTITY,
            "blocked_address",
            "link-preview-blocked-address",
        ),
        LinkPreviewFailure::DnsFailed => (
            StatusCode::BAD_GATEWAY,
            "dns_failed",
            "link-preview-dns-failed",
        ),
        LinkPreviewFailure::UpstreamError => (
            StatusCode::BAD_GATEWAY,
            "upstream_error",
            "link-preview-upstream-error",
        ),
        LinkPreviewFailure::Timeout => (
            StatusCode::GATEWAY_TIMEOUT,
            "timeout",
            "link-preview-timeout",
        ),
        LinkPreviewFailure::NotHtml => (
            StatusCode::UNPROCESSABLE_ENTITY,
            "not_html",
            "link-preview-not-html",
        ),
        LinkPreviewFailure::NoPreview => (
            StatusCode::UNPROCESSABLE_ENTITY,
            "no_preview",
            "link-preview-no-preview",
        ),
        LinkPreviewFailure::TooManyRequests => (
            StatusCode::TOO_MANY_REQUESTS,
            "too_many_requests",
            "link-preview-too-many-requests",
        ),
        // 核心自己不会给出 `Other`，它只是 App 认不出新类别时的落脚处。
        LinkPreviewFailure::Other => (
            StatusCode::INTERNAL_SERVER_ERROR,
            "other",
            "link-preview-upstream-error",
        ),
    };
    let detail = failure.detail.as_str();
    error_response(
        status,
        kind,
        &l10n::text(&langs, message, &[("detail", detail.into())]),
    )
}

#[cfg(test)]
#[path = "link_preview_tests.rs"]
mod tests;
