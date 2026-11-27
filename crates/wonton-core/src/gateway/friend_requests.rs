//! 家外的人要加管理员批的成员时，邀请在 Tuwunel 那里就被 MSC4155 挡住（[`crate::matrix::invite_filter`]），
//! 核心看不到，成员也收不到审批。服务器名上的公开落地页补这条路：外人填自己的 Matrix ID 和对方，
//! 核心开一张管理员卡片；同意了就把这个人记进对方的 `allowed_users`，他重新发一次邀请就能进来。
//! 只服务“要管理员批”（`AskAdmin`）的角色：能自己加好友的让直接发邀请，不能加的回 409 让管理员邀请。
//!
//! 页面只用 `Host`（[`super::entry_federation`]，server_name 的入口），不需要账号和设备。公网页面，挡滥用靠
//! “同一个（对方，发起人）只留一条”和总条数上限；结果存在内存里，核心重启就没了（同卡片）。
//!
//! | 方法 | 路径 | 说明 |
//! | --- | --- | --- |
//! | GET | `/`、`/_wonton/friend-request` | 表单，`?to=` 预填对方 |
//! | POST | `/_wonton/friend-request` | 交请求，303 到这条请求的进度页 |
//! | GET | `/_wonton/friend-request/{id}` | 进度页，等的时候每几秒自己刷新 |

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::extract::{Query, Request};
use axum::response::{Html, IntoResponse, Response};
use hyper::{Method, StatusCode, header};
use serde::Deserialize;
use wonton_protocol::{Answer, CardChoice, CardLine, Gate};

use super::api::languages;
use super::pages::{ERROR_HEADER, escape, layout};
use super::{Gateway, Person};
use crate::l10n;
use crate::matrix::invite_filter::ExactUserId;
use crate::mcp::{Draft, Refusal};
use crate::plugin::languages::Languages;

/// 表单和进度页的路径。
const PATH: &str = "/_wonton/friend-request";
/// 同时等管理员回答的请求条数上限。确认总线对一个调用方也只留 3 张，这里跟着它；公网谁都能交，
/// 不限会让管理员卡片被刷满。
const MAX_PENDING: usize = 3;
/// 结果留多久（卡片最多等十分钟，留够请求方回来看）。
const KEEP: Duration = Duration::from_secs(30 * 60);
/// 进度页自己刷新的间隔。
const REFRESH_SECS: u64 = 3;

/// 一条请求最后的结果。
#[derive(Clone)]
enum Outcome {
    Approved,
    /// 等的时候对方的角色放开了，不用再记名单。
    Open,
    Denied,
    Expired,
    /// 对方被移除。
    Changed,
    /// 管理员卡片已经攒满（包括安装卡占掉的名额）。
    Busy,
    Unavailable(String),
}

struct Entry {
    target: String,
    requester: String,
    created: Instant,
    outcome: Option<Outcome>,
}

/// 还在等或刚答完的请求，按 ID 存。
#[derive(Default)]
pub(crate) struct Requests {
    entries: Mutex<BTreeMap<String, Entry>>,
}

#[derive(Deserialize)]
pub struct Submission {
    #[serde(default)]
    user_id: String,
    #[serde(default)]
    to: String,
}

#[derive(Deserialize)]
pub struct Prefill {
    #[serde(default)]
    to: String,
}

/// 公开入口上的路径：根和请求页。
pub(super) fn is_public(path: &str) -> bool {
    path == "/" || path == PATH || path.starts_with(&format!("{PATH}/"))
}

/// 一次要有几个人的名字：对方成员的显示名，用来写提示和卡片。
fn display_name(gateway: &Gateway, target: &str) -> String {
    gateway
        .members
        .get(target)
        .map(|member| member.display_name)
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| format!("@{target}"))
}

fn reply(status: StatusCode, kind: &'static str, text: String, langs: &Languages) -> Response {
    let title = l10n::text(langs, "gateway-friend-request-title", &[]);
    let body = format!(
        "<h1>{title}</h1><p>{text}</p><p class=\"muted\"><a href=\"{PATH}\">{again}</a></p>",
        title = escape(&title),
        text = escape(&text),
        again = escape(&l10n::text(langs, "gateway-friend-request-again", &[])),
    );
    let mut response = (status, Html(layout(&title, &body, langs))).into_response();
    response
        .headers_mut()
        .insert(ERROR_HEADER, header::HeaderValue::from_static(kind));
    response
}

fn page(status: StatusCode, body: String, langs: &Languages, refresh: bool) -> Response {
    let title = l10n::text(langs, "gateway-friend-request-title", &[]);
    let body = if refresh {
        format!("<meta http-equiv=\"refresh\" content=\"{REFRESH_SECS}\">{body}")
    } else {
        body
    };
    (status, Html(layout(&title, &body, langs))).into_response()
}

/// 入口分过来的公开请求。
pub(super) async fn request(gateway: Arc<Gateway>, req: Request) -> Response {
    let langs = languages(req.headers());
    let path = req.uri().path().to_owned();
    match (req.method().clone(), path.as_str()) {
        (Method::GET | Method::HEAD, "/" | PATH) => {
            let prefill = Query::<Prefill>::try_from_uri(req.uri())
                .map(|Query(prefill)| prefill)
                .unwrap_or(Prefill { to: String::new() });
            form(&gateway, &langs, &prefill.to)
        }
        (Method::POST, PATH) => submit(gateway, req, langs).await,
        (Method::GET | Method::HEAD, path) if path.starts_with(&format!("{PATH}/")) => {
            let id = path.trim_start_matches(&format!("{PATH}/"));
            status(&gateway, &langs, id)
        }
        _ => {
            let mut response = reply(
                StatusCode::METHOD_NOT_ALLOWED,
                "invalid_method",
                l10n::text(&langs, "gateway-friend-request-invalid-method", &[]),
                &langs,
            );
            response
                .headers_mut()
                .insert(header::ALLOW, header::HeaderValue::from_static("GET, POST"));
            response
        }
    }
}

fn form(gateway: &Gateway, langs: &Languages, to: &str) -> Response {
    let to = to.trim();
    let target = local_member(gateway, to);
    let intro = l10n::text(langs, "gateway-friend-request-intro", &[]);
    let target_field = match &target {
        Some(member) => format!(
            "<p>{}</p><input type=\"hidden\" name=\"to\" value=\"{}\">",
            escape(&l10n::text(
                langs,
                "gateway-friend-request-target",
                &[("target", display_name(gateway, member).as_str().into())],
            )),
            escape(to),
        ),
        None => format!(
            "<p><label>{}</label><input name=\"to\" value=\"{}\" placeholder=\"@bob:example.org\" required></p>",
            escape(&l10n::text(langs, "gateway-friend-request-to", &[])),
            escape(to),
        ),
    };
    let body = format!(
        "<h1>{}</h1><p>{}</p><form method=\"post\" action=\"{PATH}\">\
         <p><label>{label}</label><input name=\"user_id\" autocomplete=\"username\" placeholder=\"@you:example.org\" required autofocus></p>\
         {target_field}\
         <p><button type=\"submit\">{submit}</button></p></form>",
        escape(&l10n::text(langs, "gateway-friend-request-title", &[])),
        escape(&intro),
        label = escape(&l10n::text(langs, "gateway-friend-request-you", &[])),
        submit = escape(&l10n::text(langs, "gateway-friend-request-submit", &[])),
    );
    page(StatusCode::OK, body, langs, false)
}

/// 对方是不是这台服务器上还留着的成员；返回用户名。
fn local_member<'a>(gateway: &Gateway, user_id: &'a str) -> Option<&'a str> {
    let server = gateway.matrix.as_ref().map(|matrix| matrix.server_name())?;
    let (localpart, host) = user_id.trim().split_once(':')?;
    if !localpart.starts_with('@') || !host.eq_ignore_ascii_case(server) {
        return None;
    }
    let localpart = &localpart[1..];
    if localpart == gateway.owner.username() {
        return Some(localpart);
    }
    gateway
        .members
        .get(localpart)
        .filter(|member| member.status != wonton_protocol::MemberStatus::Removed)
        .map(|_| localpart)
}

async fn submit(gateway: Arc<Gateway>, req: Request, langs: Languages) -> Response {
    let bytes = match axum::body::to_bytes(req.into_body(), 64 * 1024).await {
        Ok(bytes) => bytes,
        Err(_) => return invalid(&langs),
    };
    let Ok(Submission { user_id, to }) = serde_urlencoded::from_bytes::<Submission>(&bytes) else {
        return invalid(&langs);
    };
    let (requester, target) = match check(&gateway, user_id.trim(), to.trim()) {
        Ok(pair) => pair,
        Err(reason) => return refused(reason, &langs),
    };
    let Some(bus) = gateway.mcp.as_ref().map(|endpoint| endpoint.bus.clone()) else {
        return reply(
            StatusCode::SERVICE_UNAVAILABLE,
            "confirmation_unavailable",
            l10n::text(&langs, "gateway-friend-request-unavailable", &[]),
            &langs,
        );
    };
    let id = match gateway.friend_requests.reserve(&target, requester.as_str()) {
        Ok(id) => id,
        Err(err) => {
            let (status, kind, text) = match err {
                Reserve::Duplicate => (
                    StatusCode::CONFLICT,
                    "duplicate",
                    l10n::text(&langs, "gateway-friend-request-duplicate", &[]),
                ),
                Reserve::TooMany => (
                    StatusCode::TOO_MANY_REQUESTS,
                    "too_many",
                    l10n::text(&langs, "gateway-friend-request-too-many", &[]),
                ),
            };
            return reply(status, kind, text, &langs);
        }
    };
    // 卡片收件人是管理员：按主人的语言（ADR-0043，同出站卡片）；页面本身仍跟游客的 Accept-Language。
    let draft = draft(&gateway, &gateway.plugins.owner_languages(), &target);
    let location = format!("{PATH}/{id}");
    let asking = async move {
        let outcome = match bus
            .ask_plugin_admin(requester.as_str().to_owned(), draft)
            .await
        {
            Ok(_ticket) => approve(&gateway, &target, &requester).await,
            Err(Refusal::Rejected) => Outcome::Denied,
            Err(Refusal::Expired) => Outcome::Expired,
            Err(Refusal::TooMany) => Outcome::Busy,
            Err(Refusal::Unavailable(message)) => Outcome::Unavailable(message),
            Err(other) => Outcome::Unavailable(other.message()),
        };
        gateway.friend_requests.finish(&id, outcome);
    };
    tokio::spawn(asking);
    let mut response = StatusCode::SEE_OTHER.into_response();
    response.headers_mut().insert(
        header::LOCATION,
        header::HeaderValue::from_str(&location).expect("路径合法"),
    );
    response
}

fn invalid(langs: &Languages) -> Response {
    reply(
        StatusCode::BAD_REQUEST,
        "invalid_input",
        l10n::text(langs, "gateway-friend-request-invalid", &[]),
        langs,
    )
}

/// 两个 ID 都过关才返回（发起人，对方成员用户名）；拒绝的原因小，响应到调用处再拼。发起人要是一个具体的
/// Matrix 用户（[`ExactUserId`]）：批准会把它原样写进名单。
fn check(gateway: &Gateway, requester: &str, to: &str) -> Result<(ExactUserId, String), Refused> {
    let Some(target) = local_member(gateway, to).map(str::to_owned) else {
        return Err(Refused::Unknown);
    };
    let Some(requester) = ExactUserId::parse(requester) else {
        return Err(Refused::Invalid);
    };
    let server = gateway
        .matrix
        .as_ref()
        .map(|matrix| matrix.server_name())
        .unwrap_or_default();
    if requester.server_name().eq_ignore_ascii_case(server) {
        return Err(Refused::Local);
    }
    if target == gateway.owner.username() {
        return Err(Refused::Open);
    }
    match gateway.permissions(&Person::member(&target)).add_friends {
        Gate::Allow => Err(Refused::Open),
        // “朋友”模板（以及不认识、按 Deny 处理的取值）不能自己加好友，邀请会被拦在这里，页面也帮不上。
        Gate::Deny | Gate::Other => Err(Refused::Forbidden),
        Gate::AskAdmin => Ok((requester, target)),
    }
}

/// `check` 的拒绝原因。
enum Refused {
    Unknown,
    Invalid,
    Local,
    Open,
    Forbidden,
}

fn refused(refused: Refused, langs: &Languages) -> Response {
    match refused {
        Refused::Unknown => reply(
            StatusCode::NOT_FOUND,
            "invalid_input",
            l10n::text(langs, "gateway-friend-request-unknown", &[]),
            langs,
        ),
        Refused::Invalid => invalid(langs),
        Refused::Local => reply(
            StatusCode::CONFLICT,
            "not_needed",
            l10n::text(langs, "gateway-friend-request-local", &[]),
            langs,
        ),
        Refused::Open => reply(
            StatusCode::CONFLICT,
            "not_needed",
            l10n::text(langs, "gateway-friend-request-open", &[]),
            langs,
        ),
        Refused::Forbidden => reply(
            StatusCode::CONFLICT,
            "forbidden",
            l10n::text(langs, "gateway-friend-request-forbidden", &[]),
            langs,
        ),
    }
}

/// 管理员卡片：第一行是发起人的 ID（总线加的），正文说想加谁。
fn draft(gateway: &Gateway, langs: &Languages, target: &str) -> Draft {
    Draft {
        title: l10n::text(langs, "gateway-friend-request-card-title", &[]),
        lines: vec![CardLine {
            text: l10n::text(
                langs,
                "gateway-friend-request-card-to",
                &[("target", display_name(gateway, target).as_str().into())],
            ),
            emphasis: false,
        }],
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

/// 批准之后的落账：在锁里重看这个成员，还受限才记名单。
async fn approve(gateway: &Gateway, target: &str, requester: &ExactUserId) -> Outcome {
    let _sync = gateway.member_sync.lock().await;
    let Some(record) = gateway
        .members
        .get(target)
        .filter(|member| member.status != wonton_protocol::MemberStatus::Removed)
    else {
        return Outcome::Changed;
    };
    match gateway.permissions(&Person::member(target)).add_friends {
        // 等的时候角色放开了就不用再记名单；收成不能加好友的，这次批了也没用。
        Gate::Allow => return Outcome::Open,
        Gate::Deny | Gate::Other => return Outcome::Changed,
        Gate::AskAdmin => {}
    }
    let Some(matrix) = gateway.matrix.as_deref() else {
        return Outcome::Unavailable("没有接上 Matrix".to_owned());
    };
    match matrix
        .allow_invitees(&record.username, std::slice::from_ref(requester))
        .await
    {
        Ok(()) => Outcome::Approved,
        Err(error) => Outcome::Unavailable(format!("没能记下这个人：{error}")),
    }
}

fn status(gateway: &Gateway, langs: &Languages, id: &str) -> Response {
    let Some(entry) = gateway.friend_requests.get(id) else {
        return reply(
            StatusCode::NOT_FOUND,
            "not_found",
            l10n::text(langs, "gateway-friend-request-unknown", &[]),
            langs,
        );
    };
    let waiting = format!(
        "<p>{}</p><p class=\"muted\">{}</p>",
        escape(&l10n::text(
            langs,
            "gateway-friend-request-waiting",
            &[(
                "target",
                display_name(gateway, &entry.target).as_str().into()
            )],
        )),
        escape(&entry.requester),
    );
    match &entry.outcome {
        None => page(StatusCode::OK, waiting, langs, true),
        Some(outcome) => {
            let (status, kind, text) = match outcome {
                Outcome::Approved => (
                    StatusCode::OK,
                    "approved",
                    l10n::text(langs, "gateway-friend-request-approved", &[]),
                ),
                Outcome::Open => (
                    StatusCode::OK,
                    "open",
                    l10n::text(langs, "gateway-friend-request-open", &[]),
                ),
                Outcome::Denied => (
                    StatusCode::FORBIDDEN,
                    "denied",
                    l10n::text(langs, "gateway-friend-request-denied", &[]),
                ),
                Outcome::Expired => (
                    StatusCode::REQUEST_TIMEOUT,
                    "expired",
                    l10n::text(langs, "gateway-friend-request-expired", &[]),
                ),
                Outcome::Changed => (
                    StatusCode::FORBIDDEN,
                    "permission_denied",
                    l10n::text(langs, "gateway-friend-request-changed", &[]),
                ),
                Outcome::Busy => (
                    StatusCode::TOO_MANY_REQUESTS,
                    "too_many",
                    l10n::text(langs, "gateway-friend-request-too-many", &[]),
                ),
                Outcome::Unavailable(message) => (
                    StatusCode::SERVICE_UNAVAILABLE,
                    "confirmation_unavailable",
                    format!(
                        "{}（{message}）",
                        l10n::text(langs, "gateway-friend-request-unavailable", &[])
                    ),
                ),
            };
            reply(status, kind, text, langs)
        }
    }
}

/// 登记一条待批；同一个人对同一个人只留一条。
enum Reserve {
    Duplicate,
    TooMany,
}

impl Requests {
    fn reserve(&self, target: &str, requester: &str) -> Result<String, Reserve> {
        let mut entries = self.entries.lock().expect("请求表的锁中毒");
        entries.retain(|_, entry| entry.created.elapsed() < KEEP);
        if entries.values().any(|entry| {
            entry.outcome.is_none() && entry.target == target && entry.requester == requester
        }) {
            return Err(Reserve::Duplicate);
        }
        if entries
            .values()
            .filter(|entry| entry.outcome.is_none())
            .count()
            >= MAX_PENDING
        {
            return Err(Reserve::TooMany);
        }
        let id = crate::secret::random_token();
        entries.insert(
            id.clone(),
            Entry {
                target: target.to_owned(),
                requester: requester.to_owned(),
                created: Instant::now(),
                outcome: None,
            },
        );
        Ok(id)
    }

    fn finish(&self, id: &str, outcome: Outcome) {
        if let Some(entry) = self.entries.lock().expect("请求表的锁中毒").get_mut(id) {
            entry.outcome = Some(outcome);
        }
    }

    fn get(&self, id: &str) -> Option<Entry> {
        let entries = self.entries.lock().expect("请求表的锁中毒");
        entries.get(id).map(|entry| Entry {
            target: entry.target.clone(),
            requester: entry.requester.clone(),
            created: entry.created,
            outcome: entry.outcome.clone(),
        })
    }
}
