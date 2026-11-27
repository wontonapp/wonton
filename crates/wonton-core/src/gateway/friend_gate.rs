//! 出站 Matrix 邀请按邀请人的“加好友”权限项判断（ADR-0088 第 2、3 节）。邀请 Matrix 上的人进私聊或群
//! 都经过这里（几种写法见 [`Shape`]）；家里人（同一台服务器上的用户）不受限。`Allow` 放行，`Deny` 和
//! 不认识的取值回 403，`AskAdmin` 记一张管理员卡片（谁先答算谁的）并挂住这次请求：同意后在锁里复核
//! 设备归属和当前角色，把这次获批的人写进邀请人的 `allowed_users`，再按原请求转发给 Tuwunel；拒绝、
//! 过期按 403、408 回给邀请人的设备。设备断开不影响卡片，管理员批准后邀请照样发出。

use std::sync::Arc;

use axum::body::{Body, to_bytes};
use axum::extract::Request;
use axum::response::Response;
use hyper::{Method, StatusCode};
use serde::Deserialize;
use wonton_protocol::{Answer, CardChoice, CardLine, Gate};

use super::api::acted_by;
use super::matrix::{deliver_client_request, matrix_error};
use super::{Gateway, Person, TunnelPeer};
use crate::l10n;
use crate::matrix::invite_filter::ExactUserId;
use crate::mcp::{Draft, Refusal};
use crate::plugin::languages::Languages;

/// 看邀请请求体时的上限；超过就看不出邀请了谁，回 413，不放过去（同 `people_limits`）。
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
    let Some(shape) = Shape::of(req.method(), req.uri().path()) else {
        return Outcome::Pass(req);
    };
    let (parts, body) = req.into_parts();
    let Ok(bytes) = to_bytes(body, MAX_BODY).await else {
        return Outcome::Stop(matrix_error(
            StatusCode::PAYLOAD_TOO_LARGE,
            "M_TOO_LARGE",
            "invalid_input",
            "请求体太大，看不出邀请了谁",
        ));
    };
    let remote: Vec<ExactUserId> = match shape.invitees(&bytes) {
        Ok(invitees) => invitees
            .into_iter()
            .filter(|user| user.server_name() != matrix.server_name())
            .collect(),
        Err(refused) => return Outcome::Stop(*refused),
    };
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

/// 能发出邀请的写法。Tuwunel v2.0.0 对后两种的远端目标不查 MSC4155（`api/client/state.rs` 的
/// `validate_member` 只查本服务器的），照样把成员事件发给对方服务器（`timeline/build.rs`），所以都要过门。
enum Shape {
    /// `POST …/rooms/{房间}/invite`
    Invite,
    /// `POST …/createRoom`：`invite` 名单和 `initial_state` 里的成员事件。
    CreateRoom,
    /// `PUT …/rooms/{房间}/state/m.room.member/{对方}`，带解码后的状态键。
    MemberState(String),
}

impl Shape {
    fn of(method: &Method, path: &str) -> Option<Self> {
        let rest = path.strip_prefix("/_matrix/client/")?;
        let mut segments = rest.split('/');
        // 版本段（v3、r0）非空就行。
        segments.next().filter(|version| !version.is_empty())?;
        match (method, segments.next()?) {
            (&Method::POST, "createRoom") => Some(Self::CreateRoom),
            (method, "rooms") => {
                segments.next().filter(|room| !room.is_empty())?;
                match (method, segments.next()?) {
                    (&Method::POST, "invite") => Some(Self::Invite),
                    (&Method::PUT, "state") if decoded(segments.next()?) == "m.room.member" => {
                        let key: Vec<&str> = segments.collect();
                        Some(Self::MemberState(decoded(&key.join("/"))))
                    }
                    _ => None,
                }
            }
            _ => None,
        }
    }

    /// 这次邀请了谁。空请求体按 `{}`（ruma 的读法）；JSON 读不出来回 400 `M_BAD_JSON`，邀请的不是一个
    /// 具体用户（通配、超长、格式不对，见 [`ExactUserId`]）回 400 `M_INVALID_PARAM`，都不开卡。字段按
    /// 结构体读：重复的键报错而不是取最后一个，和 Tuwunel 的读法一致，免得两边看到的邀请对象不一样。
    fn invitees(&self, body: &[u8]) -> Result<Vec<ExactUserId>, Box<Response>> {
        let body = if body.is_empty() {
            b"{}".as_slice()
        } else {
            body
        };
        let raw: Vec<String> = match self {
            Self::Invite => json::<InviteBody>(body)?.user_id.into_iter().collect(),
            Self::CreateRoom => {
                let room = json::<CreateRoomBody>(body)?;
                let members = room.initial_state.into_iter().filter_map(|event| {
                    (event.kind == "m.room.member" && event.content.is_invite())
                        .then_some(event.state_key)
                });
                room.invite.into_iter().chain(members).collect()
            }
            Self::MemberState(target) => {
                if json::<MemberContent>(body)?.is_invite() {
                    vec![target.clone()]
                } else {
                    Vec::new()
                }
            }
        };
        raw.iter()
            .map(|user| {
                ExactUserId::parse(user).ok_or_else(|| {
                    Box::new(matrix_error(
                        StatusCode::BAD_REQUEST,
                        "M_INVALID_PARAM",
                        "invalid_input",
                        "邀请的不是一个具体的 Matrix 用户 ID",
                    ))
                })
            })
            .collect()
    }
}

#[derive(Deserialize)]
struct InviteBody {
    user_id: Option<String>,
}

#[derive(Deserialize)]
struct CreateRoomBody {
    #[serde(default)]
    invite: Vec<String>,
    #[serde(default)]
    initial_state: Vec<InitialState>,
}

#[derive(Deserialize)]
struct InitialState {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    state_key: String,
    #[serde(default)]
    content: MemberContent,
}

#[derive(Deserialize, Default)]
struct MemberContent {
    membership: Option<String>,
}

impl MemberContent {
    fn is_invite(&self) -> bool {
        self.membership.as_deref() == Some("invite")
    }
}

fn json<T: serde::de::DeserializeOwned>(body: &[u8]) -> Result<T, Box<Response>> {
    serde_json::from_slice(body).map_err(|_| {
        Box::new(matrix_error(
            StatusCode::BAD_REQUEST,
            "M_BAD_JSON",
            "invalid_input",
            "邀请的请求体不是合法的 JSON",
        ))
    })
}

fn decoded(segment: &str) -> String {
    String::from_utf8_lossy(&super::matrix::percent_decode(segment)).into_owned()
}

/// `AskAdmin`：开卡片、挂住请求；批准后按原请求（同一个人、同一个访问令牌）把邀请发出去。
fn hold(
    gateway: &Arc<Gateway>,
    parts: hyper::http::request::Parts,
    bytes: axum::body::Bytes,
    person: Person,
    peer: Option<TunnelPeer>,
    invitees: Vec<ExactUserId>,
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
fn invite_card(langs: &Languages, inviter: &str, invitees: &[ExactUserId]) -> Draft {
    Draft {
        title: l10n::text(langs, "gateway-friend-invite-title", &[]),
        lines: invitees
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
    invitees: Vec<ExactUserId>,
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
