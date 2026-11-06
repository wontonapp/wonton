//! `health.wonton.internal`：申请了 `plugins.health` 的插件读所有插件的状态，详情接口由核心当场探测目标
//! 插件的网页入口（规范“插件健康探测”）。按来源地址认出请求来自哪个插件网络（同 `mcp.wonton.internal`），
//! 只读，不带主人身份。

use std::collections::HashMap;
use std::sync::Mutex;

use axum::Json;
use axum::extract::Request;
use axum::response::{IntoResponse, Response};
use hyper::{Method, StatusCode};

use super::api::error_response;
use super::{Gateway, client_addr};
use crate::plugin::{Permission, PluginId};

/// 每个调用方每秒最多的请求次数（规范“插件健康探测”）。
const PER_SECOND: f64 = 5.0;

/// `plugins.health` 的限频表：每个调用方一个令牌桶，防止监控插件把核心的探测打爆。
pub(crate) struct RateLimits {
    buckets: Mutex<HashMap<PluginId, Bucket>>,
}

#[derive(Default)]
struct Bucket {
    tokens: f64,
    last: Option<std::time::Instant>,
}

impl RateLimits {
    pub(crate) fn new() -> Self {
        Self {
            buckets: Mutex::new(HashMap::new()),
        }
    }

    /// 放行这次请求；每秒额度用完时拒绝，按过了多久补回令牌。
    pub(crate) fn allow(&self, caller: &PluginId) -> bool {
        let mut buckets = self.buckets.lock().expect("限频表锁中毒");
        let now = std::time::Instant::now();
        let bucket = buckets.entry(caller.clone()).or_default();
        let tokens = match bucket.last {
            Some(last) => (bucket.tokens + PER_SECOND * now.duration_since(last).as_secs_f64())
                .min(PER_SECOND),
            None => PER_SECOND,
        };
        bucket.last = Some(now);
        if tokens >= 1.0 {
            bucket.tokens = tokens - 1.0;
            true
        } else {
            bucket.tokens = tokens;
            false
        }
    }
}

pub async fn request(gateway: &Gateway, req: Request) -> Response {
    let source = match client_addr(&req).and_then(|addr| gateway.plugins.source(addr.ip())) {
        Some(source) if source.permissions.contains(&Permission::PluginsHealth) => source,
        Some(source) => {
            return error_response(
                StatusCode::FORBIDDEN,
                "permission_denied",
                &format!("插件 {} 没有申请 plugins.health 权限", source.id),
            );
        }
        None => {
            return error_response(
                StatusCode::FORBIDDEN,
                "unauthenticated",
                "这个地址只给插件网络里的插件用",
            );
        }
    };
    if !gateway.health_limits.allow(&source.id) {
        return error_response(
            StatusCode::TOO_MANY_REQUESTS,
            "too_many_requests",
            "探测请求太频繁：这个地址每秒最多 5 次，请降低监控频率",
        );
    }
    match (req.method(), req.uri().path()) {
        (&Method::GET, "/plugins") => Json(gateway.plugins.health_reports()).into_response(),
        (&Method::GET, path) => match path.strip_prefix("/plugins/").filter(|id| !id.is_empty()) {
            Some(id) => match PluginId::parse(id) {
                Ok(id) => match gateway.plugins.health_report(&id).await {
                    Some(report) => Json(report).into_response(),
                    None => not_installed(id.as_str()),
                },
                Err(_) => not_installed(id),
            },
            None => error_response(
                StatusCode::NOT_FOUND,
                "not_found",
                "只有 GET /plugins 和 GET /plugins/<id> 两个接口",
            ),
        },
        _ => error_response(
            StatusCode::METHOD_NOT_ALLOWED,
            "method_not_allowed",
            "这个地址只支持 GET",
        ),
    }
}

fn not_installed(id: &str) -> Response {
    error_response(
        StatusCode::NOT_FOUND,
        "not_installed",
        &format!("没有安装 {id} 这个插件"),
    )
}
