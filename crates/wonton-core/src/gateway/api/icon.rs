use std::sync::Arc;

use axum::Extension;
use axum::extract::{Path, State};
use axum::response::{IntoResponse, Response};
use hyper::header::{CACHE_CONTROL, CONTENT_TYPE};
use hyper::{HeaderMap, StatusCode};

use super::{Gateway, TunnelPeer, error_response, languages, plugin_denied, text1};
use crate::l10n;
use crate::plugin::gallery::Variant;
use crate::plugin::image_worker::ImageError;

/// 插件的图标。地址里有原文件的 sha256，同一个地址的内容不变，所以可以一直缓存。
pub async fn image(
    State(gateway): State<Arc<Gateway>>,
    peer: Option<Extension<TunnelPeer>>,
    headers: HeaderMap,
    Path((sha256, file)): Path<(String, String)>,
) -> Response {
    if let Some(denied) = plugin_denied(&gateway, peer, &headers) {
        return denied;
    }
    let langs = languages(&headers);
    let icon = l10n::text(&langs, "gateway-icon-not-found", &[]);
    let not_found = || error_response(StatusCode::NOT_FOUND, "not_found", &icon);
    let Some(variant) = Variant::parse(&file) else {
        return not_found();
    };
    match gateway.plugins.icon_png(&sha256, variant).await {
        Ok(Some(png)) => (
            [
                (CONTENT_TYPE, "image/png"),
                (CACHE_CONTROL, "private, max-age=31536000, immutable"),
            ],
            png,
        )
            .into_response(),
        Ok(None) => not_found(),
        Err(ImageError::Refused(reason) | ImageError::TimedOut(reason)) => error_response(
            StatusCode::UNPROCESSABLE_ENTITY,
            "invalid_input",
            &text1(&langs, "gateway-icon-failed", reason.as_str()),
        ),
        Err(e @ ImageError::Unavailable(_)) => error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "images_unavailable",
            &e.to_string(),
        ),
    }
}
