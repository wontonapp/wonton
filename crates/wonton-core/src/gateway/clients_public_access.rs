//! 公网访问总开关（ADR-0090 第 2 节）：默认关，只有主人能开，关着时公网入口全部拒绝。
//!
//! 开关把三道闸门一起拨：核心在 8484 上拒绝 `X-Wonton-Entry: public`、入口容器的公网
//! crt-list 清空、公网地址不再有可握手的 SNI。关不干净的如实回 503，入口运行循环会按新状态重试。

use std::sync::Arc;

use axum::Router;
use axum::extract::{Json, State};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use hyper::header::HeaderMap;
use wonton_protocol::{ClientsFailure, PublicAccess, clients_public_access_path};

use super::Gateway;
use super::api;
use super::clients_api::{failure, no_store};
use crate::clients::{apply, public_access};

pub(super) fn routes() -> Router<Arc<Gateway>> {
    Router::new().route(&clients_public_access_path(), get(read).put(save))
}

async fn read(State(gateway): State<Arc<Gateway>>) -> Response {
    no_store(
        Json(PublicAccess {
            enabled: gateway.public_access_enabled(),
        })
        .into_response(),
    )
}

async fn save(
    State(gateway): State<Arc<Gateway>>,
    headers: HeaderMap,
    Json(request): Json<PublicAccess>,
) -> Response {
    let langs = api::languages(&headers);
    let data_dir = gateway.plugins.data_dir();
    if let Err(error) = public_access::save(&data_dir, request.enabled) {
        return failure(
            ClientsFailure::IngressUnavailable,
            &crate::l10n::text(
                &langs,
                "clients-public-access-store",
                &[("error", error.to_string().as_str().into())],
            ),
        );
    }
    // 先记状态再拨闸门：闸门失败时状态已经落盘，运行循环按新状态重试。
    gateway.set_public_access(request.enabled);
    if let Err(error) = apply::reapply_public_access(&gateway).await {
        return failure(
            ClientsFailure::IngressUnavailable,
            &crate::l10n::text(
                &langs,
                "clients-public-access-apply",
                &[("error", error.localized_message(&langs).as_str().into())],
            ),
        );
    }
    no_store(Json(request).into_response())
}
