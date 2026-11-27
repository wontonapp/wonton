//! 邀请过滤账号数据的设备侧看护（ADR-0088 第 9 节）。“加好友”要管理员批或不能的成员，账号数据里的
//! MSC4155 名单由核心按角色写：放本服务器、挡家外。他的设备改或删这份账号数据（自己的屏蔽写的就是这个事件）
//! 会把核心写的名单整份抹掉，让家外的邀请重新进来，所以网关拒绝：403 `M_FORBIDDEN`。主人和“加好友”
//! 放开的角色不受影响，账号数据照旧归他自己的客户端管。只看全局账号数据；房间账号数据 Tuwunel 的过滤
//! 不读，放行。

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

/// 改全局账号数据里的邀请过滤：读（GET、HEAD、OPTIONS）以外的方法都算。路径结尾是
/// `user/{用户}/account_data/{类型}`，前面是版本段（`v3`、`r0`），或 MSC3391 删除接口的
/// `unstable/org.matrix.msc3391`（Tuwunel v2.0.0 实现了它，只校验是本人）；前缀不挑，只看结尾四段，
/// 每段先百分号解码再比。
fn is_filter_write(method: &Method, path: &str) -> bool {
    if matches!(*method, Method::GET | Method::HEAD | Method::OPTIONS) {
        return false;
    }
    let Some(rest) = path.strip_prefix("/_matrix/client/") else {
        return false;
    };
    let segments: Vec<Vec<u8>> = rest
        .trim_end_matches('/')
        .split('/')
        .map(super::matrix::percent_decode)
        .collect();
    match segments.as_slice() {
        [prefix @ .., user, owner, account_data, kind] => {
            !prefix.is_empty()
                && user == b"user"
                && !owner.is_empty()
                && account_data == b"account_data"
                && kind == INVITE_FILTER_EVENT.as_bytes()
        }
        _ => false,
    }
}
