//! 原生浏览的转发（ADR-0113 第 9、10 节）：相册的 `/library/…`、影视的 `/catalog/…`。核心按操作和种类选提供方，
//! 填 `X-Wonton-User`、换上提供方签的凭据转过去；不缓存、不落盘、不改写回包，`Range` 原样过。
//!
//! 路径照客户端发来的原样转（编号里转义过的 `/` 不解开），只去掉核心自己用的 `source`。

use std::sync::Arc;

use axum::Extension;
use axum::body::Body;
use axum::extract::{Request, State};
use axum::response::Response;
use hyper::Method;
use percent_encoding::percent_decode_str;
use wonton_protocol::media::api::MEDIA_API_PREFIX;
use wonton_protocol::media::{MediaFailure, MediaFailureCode, MediaKind, MediaOperation};

use super::bytes::pass;
use super::{Caller, reply};
use crate::gateway::Gateway;
use crate::plugin::media_source::failure;

/// 一种浏览：操作的线上取值和它服务的种类。
struct Browse {
    operation: &'static str,
    kind: MediaKind,
}

const LIBRARY: Browse = Browse {
    operation: "library",
    kind: MediaKind::Photo,
};

const CATALOG: Browse = Browse {
    operation: "catalog",
    kind: MediaKind::Video,
};

pub(super) async fn library(
    State(gateway): State<Arc<Gateway>>,
    Extension(caller): Extension<Caller>,
    request: Request,
) -> Response {
    browse(&gateway, &caller, &LIBRARY, request).await
}

pub(super) async fn catalog(
    State(gateway): State<Arc<Gateway>>,
    Extension(caller): Extension<Caller>,
    request: Request,
) -> Response {
    browse(&gateway, &caller, &CATALOG, request).await
}

async fn browse(gateway: &Gateway, caller: &Caller, what: &Browse, request: Request) -> Response {
    let (parts, body) = request.into_parts();
    let forwarded = async {
        let (path, source) = provider_target(parts.uri.path(), parts.uri.query())?;
        let route = caller
            .media
            .providers
            .route(
                &caller.person,
                caller.allowed(gateway),
                &operation(what.operation),
                Some(&what.kind),
                source.as_deref(),
            )
            .await?;
        let body = if parts.method == Method::GET {
            Body::empty()
        } else {
            body
        };
        route
            .client
            .forward(
                &caller.person,
                parts.method.clone(),
                &path,
                pass(&parts.headers),
                body,
            )
            .await
    };
    match forwarded.await {
        Ok(response) => response,
        Err(failure) => reply(&failure),
    }
}

/// 按线上取值解析，同清单和运行时自报走的是同一条解析：认得的落到变体，新操作落 `Other`，两边比得上。
fn operation(raw: &str) -> MediaOperation {
    serde_json::from_value(serde_json::Value::from(raw))
        .unwrap_or_else(|_| MediaOperation::Other(raw.to_owned()))
}

/// 去掉核心前缀的提供方路径（原样转义）和查询串里的 `source`。`.`、`..`（含转义写法）、空段回 `protocol`：
/// 转过去的路径不能跑出这种浏览的前缀。
fn provider_target(
    path: &str,
    query: Option<&str>,
) -> Result<(String, Option<String>), MediaFailure> {
    let path = path.strip_prefix(MEDIA_API_PREFIX).unwrap_or(path);
    // 根路径结尾的 `/` 不算空段：`/library/` 就是 `/library`。
    let path = path
        .strip_suffix('/')
        .filter(|root| root.get(1..).is_some_and(|rest| !rest.contains('/')))
        .unwrap_or(path);
    let bad = || {
        failure(
            MediaFailureCode::Protocol,
            format!("浏览的路径不合法：{path}"),
        )
    };
    for piece in path.split('/').skip(1) {
        let decoded = percent_decode_str(piece).decode_utf8().map_err(|_| bad())?;
        if matches!(decoded.as_ref(), "" | "." | "..") {
            return Err(bad());
        }
    }
    let mut source = None;
    let mut kept = Vec::new();
    for pair in query
        .unwrap_or_default()
        .split('&')
        .filter(|pair| !pair.is_empty())
    {
        match pair.strip_prefix("source=") {
            Some(value) => {
                // `source=` 空着当没带，照默认规则选。
                let value = percent_decode_str(value).decode_utf8().map_err(|_| bad())?;
                source = (!value.is_empty()).then(|| value.into_owned());
            }
            None => kept.push(pair),
        }
    }
    let target = if kept.is_empty() {
        path.to_owned()
    } else {
        format!("{path}?{}", kept.join("&"))
    };
    Ok((target, source))
}
