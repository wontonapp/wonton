//! 邀请过滤账号数据的设备侧看护（ADR-0088 第 9 节）。“加好友”要管理员批或不能的成员，账号数据里的
//! MSC4155 名单由核心按角色写：放本服务器、挡家外。他的设备改这份账号数据（自己的屏蔽写的就是这个事件）
//! 会把核心写的名单整份抹掉，让家外的邀请重新进来，所以网关拒绝：403 `M_FORBIDDEN`。主人和“加好友”
//! 放开的角色不受影响，账号数据照旧归他自己的客户端管。只看全局账号数据的写入口；房间账号数据 Tuwunel
//! 的过滤不读，放行。

use axum::extract::Request;
use hyper::{Method, StatusCode};
use wonton_protocol::{Gate, INVITE_FILTER_EVENT};

use super::Gateway;
use super::matrix::matrix_error;

/// 检查要写邀请过滤账号数据的请求；放行时原样交回，受限成员的写给出要回的响应。
pub(super) fn check(
    gateway: &Gateway,
    req: Request,
) -> Result<Request, Box<axum::response::Response>> {
    if !is_filter_write(req.method(), req.uri().path()) {
        return Ok(req);
    }
    // 人认不出时交给后面的身份检查报错；这里只挡认得出又越权的。
    let Some(person) = gateway.device_person(req.extensions().get().copied()) else {
        return Ok(req);
    };
    if !matches!(gateway.permissions(&person).add_friends, Gate::Allow) {
        return Err(Box::new(matrix_error(
            StatusCode::FORBIDDEN,
            "M_FORBIDDEN",
            "permission_denied",
            "你的角色不能改邀请过滤；要改先请主人改角色",
        )));
    }
    Ok(req)
}

/// `PUT /_matrix/client/{版本}/user/{用户}/account_data/{类型}`，类型是 MSC4155 的邀请过滤。
fn is_filter_write(method: &Method, path: &str) -> bool {
    if *method != Method::PUT {
        return false;
    }
    let Some(rest) = path.strip_prefix("/_matrix/client/") else {
        return false;
    };
    let mut segments = rest.split('/');
    // 版本段（v3、r0）非空就行。
    if segments.next().is_none_or(|version| version.is_empty()) {
        return false;
    }
    segments.next() == Some("user")
        && segments.next().is_some_and(|user| !user.is_empty())
        && segments.next() == Some("account_data")
        && segments
            .next()
            .and_then(|kind| String::from_utf8(super::matrix::percent_decode(kind)).ok())
            .is_some_and(|kind| kind == INVITE_FILTER_EVENT)
        && segments.next().is_none()
}
