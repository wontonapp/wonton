//! 出站 Matrix 邀请按邀请人的“加好友”权限项判断（ADR-0088 第 2、3 节）。邀请 Matrix 上的人进私聊或群
//! 都经过这里；家里人（同一台服务器上的用户）不受限。`Allow` 放行，`Deny` 和不认识的取值回 403，
//! `AskAdmin` 记一张管理员卡片（谁先答算谁的）并挂住这次请求：同意后在锁里复核设备归属和当前角色，
//! 把这次获批的人写进邀请人的 `allowed_users`，再按原请求转发给 Tuwunel；拒绝、过期按 403、408 回给
//! 邀请人的设备。设备断开不影响卡片，管理员批准后邀请照样发出。

use std::sync::Arc;

use axum::body::{Body, to_bytes};
use axum::extract::Request;
use axum::response::Response;
use hyper::{Method, StatusCode};
use wonton_protocol::{Answer, CardChoice, CardLine, Gate};

use super::api::acted_by;
use super::matrix::{deliver_client_request, matrix_error};
use super::{Gateway, Person, TunnelPeer};
use crate::l10n;
use crate::mcp::{Draft, Refusal};
use crate::plugin::languages::Languages;

/// 看 `createRoom` 请求体时的上限；超过按“不算加好友”放行，和 `people_limits` 一样不猜。
const MAX_BODY: usize = 1024 * 1024;

pub(crate) enum Outcome {
    /// 不归这道门槛管（不是受管的邀请、邀请的全是家里人、放行）：请求带着原 body 继续走。
    Pass(Request),
    Stop(Response),
    /// 卡片已经开出去了：等管理员答，答完的 Response 从这里交回还开着的那条 HTTP 连接。
    Hold(tokio::sync::oneshot::Receiver<Response>),
}

/// 设备入口的 Matrix 请求：受管的邀请先过“加好友”这一项，其余原样交回。
pub(crate) async fn check(gateway: &Arc<Gateway>, req: Request) -> Outcome {
    let Some(matrix) = gateway.matrix.as_deref() else {
        return Outcome::Pass(req);
    };
    let (method, path) = (req.method().clone(), req.uri().path().to_owned());
    if !is_managed_invite(&method, &path) {
        return Outcome::Pass(req);
    }
    let (parts, body) = req.into_parts();
    let bytes = match to_bytes(body, MAX_BODY).await {
        Ok(bytes) => bytes,
        Err(_) => return Outcome::Pass(Request::from_parts(parts, Body::empty())),
    };
    let invitees = parse_invitees(&method, &path, &bytes);
    let remote: Vec<String> = invitees
        .into_iter()
        .filter(|user| localpart_server(user) != matrix.server_name())
        .collect();
    let peer = parts.extensions.get::<TunnelPeer>().copied();
    // 判断阶段只借用，放行时请求连同 body 原样交回。
    enum Decision {
        Pass,
        AskAdmin(Person),
        Deny,
    }
    let decision = if remote.is_empty() {
        Decision::Pass
    } else {
        match gateway.device_person(peer) {
            None => Decision::Pass,
            Some(person) => match gateway.permissions(&person).add_friends {
                Gate::Allow => Decision::Pass,
                Gate::AskAdmin => Decision::AskAdmin(person),
                // Deny 和不认识的取值按最严处理（ADR-0088 第 2 节）。
                _ => Decision::Deny,
            },
        }
    };
    match decision {
        Decision::Pass => Outcome::Pass(Request::from_parts(parts, Body::from(bytes))),
        Decision::Deny => Outcome::Stop(matrix_error(
            StatusCode::FORBIDDEN,
            "M_FORBIDDEN",
            "permission_denied",
            "你的角色不允许加外面的好友；要加先请管理员改角色",
        )),
        Decision::AskAdmin(person) => hold(gateway, parts, bytes, person, peer, remote),
    }
}

/// `POST …/rooms/{房间}/invite` 和带邀请名单的 `createRoom`，加好友和拉人进群都走这两条。
fn is_managed_invite(method: &Method, path: &str) -> bool {
    let Some(rest) = path.strip_prefix("/_matrix/client/") else {
        return false;
    };
    let mut segments = rest.split('/');
    let Some(version) = segments.next() else {
        return false;
    };
    if version.is_empty() {
        return false;
    }
    match (method, segments.next()) {
        (&Method::POST, Some("createRoom")) => true,
        (&Method::POST, Some("rooms")) => {
            let room = segments.next().unwrap_or_default();
            !room.is_empty() && segments.next() == Some("invite")
        }
        _ => false,
    }
}

/// 邀请了谁：`invite` 的 `user_id`，或 `createRoom` 的 `invite` 名单。读不出来按“没邀请人”放行，
/// 让后面的转发给出 Matrix 本来的错误，不在这里猜。
fn parse_invitees(method: &Method, path: &str, body: &[u8]) -> Vec<String> {
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(body) else {
        return Vec::new();
    };
    let rest = path.strip_prefix("/_matrix/client/").unwrap_or_default();
    let mut segments = rest.split('/');
    let _version = segments.next();
    let into_list = |users: Vec<&str>| {
        users
            .into_iter()
            .filter(|user| user.starts_with('@') && user.contains(':'))
            .map(str::to_owned)
            .collect()
    };
    match (method, segments.next()) {
        (_, Some("createRoom")) => into_list(
            value["invite"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|user| user.as_str())
                .collect(),
        ),
        (_, Some("rooms")) => value["user_id"]
            .as_str()
            .map(|user| into_list(vec![user]))
            .unwrap_or_default(),
        _ => Vec::new(),
    }
}

fn localpart_server(user_id: &str) -> &str {
    user_id.split_once(':').map_or("", |(_, server)| server)
}

/// `AskAdmin`：开卡片、挂住请求；批准后按原请求（同一个人、同一个访问令牌）把邀请发出去。
fn hold(
    gateway: &Arc<Gateway>,
    parts: hyper::http::request::Parts,
    bytes: axum::body::Bytes,
    person: Person,
    peer: Option<TunnelPeer>,
    invitees: Vec<String>,
) -> Outcome {
    let Some(mcp) = gateway.mcp.as_ref() else {
        return Outcome::Stop(matrix_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "M_UNKNOWN",
            "confirmation_unavailable",
            "确认总线不可用，暂时发不了这次邀请",
        ));
    };
    let bus = mcp.bus.clone();
    let extension = peer.map(axum::Extension);
    let identity = acted_by(gateway, extension.as_ref(), &parts.headers);
    // 发给管理员的卡片按主人的语言（ADR-0043，同安装卡片）。
    let langs = gateway.plugins.owner_languages();
    let inviter = gateway
        .identity_of(&person)
        .map(|owner| owner.display_name)
        .unwrap_or_else(|_| person.username.clone());
    let draft = invite_card(&langs, &inviter, &invitees);
    let (sent, waited) = tokio::sync::oneshot::channel();
    let gateway = Arc::clone(gateway);
    tokio::spawn(async move {
        let response = match bus.ask_plugin_admin(identity, draft).await {
            Ok(_ticket) => after_approval(gateway, parts, bytes, person, peer, invitees).await,
            Err(refusal) => refusal_response(refusal, &langs),
        };
        let _ = sent.send(response);
    });
    Outcome::Hold(waited)
}

/// 卡片内容：谁把谁加成好友。
fn invite_card(langs: &Languages, inviter: &str, invities: &[String]) -> Draft {
    Draft {
        title: l10n::text(langs, "gateway-friend-invite-title", &[]),
        lines: invities
            .iter()
            .map(|user| CardLine {
                text: l10n::text(
                    langs,
                    "gateway-friend-invite-to",
                    &[
                        ("inviter", inviter.into()),
                        ("friend", user.as_str().into()),
                    ],
                ),
                emphasis: false,
            })
            .collect(),
        choices: vec![
            CardChoice {
                answer: Answer::Approve,
                label: l10n::text(langs, "chat-choice-allow", &[]),
                choice_id: None,
            },
            CardChoice {
                answer: Answer::Reject,
                label: l10n::text(langs, "chat-choice-deny", &[]),
                choice_id: None,
            },
        ],
        scope: None,
        plugin_id: None,
        secret_inputs: Vec::new(),
        settings: None,
        category: None,
        permissions: Vec::new(),
        devices: Vec::new(),
    }
}

/// 管理员同意之后：在锁里重新核对成员和设备，把这次获批的人写进 `allowed_users`，再放行原请求。
async fn after_approval(
    gateway: Arc<Gateway>,
    parts: hyper::http::request::Parts,
    bytes: axum::body::Bytes,
    person: Person,
    peer: Option<TunnelPeer>,
    invitees: Vec<String>,
) -> Response {
    // 复核和写名单在同一把锁里：批准后这个人记进受限成员的 `allowed_users`，之后他发来的邀请
    // 也放行（ADR-0088 第 98 行）。角色已经放开的不用记，过滤本来就撤了。
    let _sync = gateway.member_sync.lock().await;
    let permission = gateway.permissions(&person).add_friends;
    let device_kept = gateway.device_person(peer).as_ref() == Some(&person);
    let mut users = invitees.clone();
    users.sort_unstable();
    users.dedup();
    let written = if device_kept && permission != Gate::Allow {
        gateway
            .matrix
            .as_deref()
            .expect("有 Matrix 才有这张卡片")
            .allow_invitees(&person.username, &users)
            .await
    } else {
        Ok(())
    };
    match (device_kept, permission, written) {
        (false, _, _) => matrix_error(
            StatusCode::FORBIDDEN,
            "M_FORBIDDEN",
            "permission_denied",
            "成员或设备已失去加好友的权限",
        ),
        (_, _, Err(error)) => matrix_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "M_UNKNOWN",
            "invite_filter_failed",
            &format!("批准了，但没能记下这个人：{error}"),
        ),
        // 角色收紧到不能发了：人已经记下，这一次发不出去。
        (_, Gate::Allow | Gate::AskAdmin, Ok(())) => {
            let req = Request::from_parts(parts, Body::from(bytes));
            deliver_client_request(&gateway, req).await
        }
        (_, _, Ok(())) => matrix_error(
            StatusCode::FORBIDDEN,
            "M_FORBIDDEN",
            "permission_denied",
            "成员的角色不能加好友了；这个人已经记下，等他来加就行",
        ),
    }
}

/// 卡片没答成（拒绝、过期或其他）时回给邀请人的设备。
fn refusal_response(refusal: Refusal, langs: &Languages) -> Response {
    match refusal {
        Refusal::Rejected => matrix_error(
            StatusCode::FORBIDDEN,
            "M_FORBIDDEN",
            "permission_denied",
            &l10n::text(langs, "gateway-friend-invite-rejected", &[]),
        ),
        Refusal::Unavailable(message) => matrix_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "M_UNKNOWN",
            "confirmation_unavailable",
            &message,
        ),
        // 卡没开成的时候报“超时”是假原因；管理员只是答不过来，让设备过会儿重试。
        Refusal::TooMany => matrix_error(
            StatusCode::TOO_MANY_REQUESTS,
            "M_UNKNOWN",
            "too_many_pending",
            "还有太多问题在等管理员，这次加好友没能开卡",
        ),
        refusal @ (Refusal::Expired | Refusal::Superseded | Refusal::Cancelled) => matrix_error(
            StatusCode::REQUEST_TIMEOUT,
            "M_UNKNOWN",
            refusal.kind(),
            &l10n::text(langs, "gateway-friend-invite-expired", &[]),
        ),
    }
}
