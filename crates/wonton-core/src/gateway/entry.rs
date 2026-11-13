//! 8484 入口监听的分发（ADR-0021、ADR-0090）：按 `Host` 分给 Matrix 联邦（[`entry_federation`]）或
//! 插件主机名（[`clients_entry`]）。对端必须是入口容器，由 `entry_runtime` 的 `trusted_entry` 中间件
//! 把关；这个监听不经过主网关的 Cookie/设备鉴权，也绝不把请求当作主人。

use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::extract::Request;
use axum::response::Response;
use hyper::body::Body as _;
use hyper::header::{self, HeaderValue};
use hyper::{Method, StatusCode};

use super::Gateway;
use super::clients_entry;
use super::entry_federation;
use super::matrix::matrix_error;

/// 只应绑定在核心私有网络中，由 TLS 入口容器转入；不能和主网关的路由合并。
pub fn router(gateway: Arc<Gateway>) -> Router {
    Router::new().fallback(move |req: Request| {
        let gateway = gateway.clone();
        async move {
            let head = req.method() == Method::HEAD;
            let mut response = dispatch(&gateway, req).await;
            if head {
                if !response.headers().contains_key(header::CONTENT_LENGTH)
                    && let Some(length) = response.body().size_hint().exact()
                    && let Ok(value) = HeaderValue::from_str(&length.to_string())
                {
                    response.headers_mut().insert(header::CONTENT_LENGTH, value);
                }
                *response.body_mut() = Body::empty();
            }
            response
        }
    })
}

async fn dispatch(gateway: &Gateway, req: Request) -> Response {
    let Some(matrix) = &gateway.matrix else {
        return entry_federation::unavailable();
    };
    let server_name = matrix.server_name();
    if entry_federation::matches_host(&req, server_name) {
        return entry_federation::dispatch(gateway, matrix, req).await;
    }
    if let Some((label, kind)) = clients_entry::split_host(&req, server_name) {
        return clients_entry::dispatch(gateway, req, label, kind).await;
    }
    matrix_error(
        StatusCode::MISDIRECTED_REQUEST,
        "M_UNRECOGNIZED",
        "invalid_host",
        "入口只伺候本机服务器名和插件的入口名字",
    )
}
