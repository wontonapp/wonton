//! 本机每个人发邀请、发消息的频率（ADR-0051 的“挡垃圾”）。Tuwunel 只有登录失败的限频，MSC4155 只管收不管发。
//!
//! 只数设备入口（经隧道的已配对设备），Bot 有自己的 [`super::send_limits`]。按人计数，同一人多台设备共用一份；
//! 令牌桶，每分钟回补 `per_minute` 个。超了回 429 `M_LIMIT_EXCEEDED` 带 `retry_after_ms`，SDK 自己会等。
//!
//! 邀请：`POST …/rooms/{房间}/invite`，以及带 `invite` 的 `POST …/createRoom`（加好友、建私聊都走它）。
//! 消息：`PUT …/rooms/{房间}/send/{m.room.message|m.room.encrypted|m.sticker}`。同一个 txn 的重发不重复计数。

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::body::to_bytes;
use axum::extract::Request;
use axum::response::{IntoResponse, Response};
use hyper::header::HeaderValue;
use hyper::{Method, StatusCode};
use serde_json::json;

use super::matrix::percent_decode;
use super::pages;

/// 数得上的消息类型。
const MESSAGE_TYPES: [&str; 3] = ["m.room.message", "m.room.encrypted", "m.sticker"];

/// 看 `createRoom` 请求体时的上限；超过按“不算邀请”放行，和 `room_rules` 一样不猜。
const MAX_BODY: usize = 1024 * 1024;

/// 多久扫一遍没动过的桶。
const SWEEP_EVERY: Duration = Duration::from_secs(60);

/// 每个人每分钟的上限（ADR-0051）。默认按普通聊天够用、刷屏挡得住。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PeopleLimits {
    pub invites_per_minute: u32,
    pub messages_per_minute: u32,
}

impl Default for PeopleLimits {
    fn default() -> Self {
        Self {
            invites_per_minute: 20,
            messages_per_minute: 60,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Kind {
    Invite,
    Message,
}

struct Bucket {
    tokens: f64,
    last: u64,
    txns: Vec<String>,
}

pub struct PeopleLimiter {
    limits: PeopleLimits,
    now: fn() -> u64,
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    buckets: HashMap<(String, Kind), Bucket>,
    last_sweep: u64,
}

fn system_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_millis() as u64)
}

impl PeopleLimiter {
    pub fn new(limits: PeopleLimits) -> Self {
        Self::with_clock(limits, system_now)
    }

    fn with_clock(limits: PeopleLimits, now: fn() -> u64) -> Self {
        Self {
            limits,
            now,
            state: Mutex::new(State::default()),
        }
    }

    /// 设备入口的请求：数得上的先过频率，放行的原样交出（请求体重新装回去）。
    pub(crate) async fn check(&self, req: Request, person: &str) -> Result<Request, Box<Response>> {
        let Some((kind, txn)) = classify(req.method(), req.uri().path()) else {
            return Ok(req);
        };
        let mut empty_invites = false;
        let req = if kind == Kind::Invite && req.uri().path().ends_with("/createRoom") {
            let (mut parts, body) = req.into_parts();
            let bytes = to_bytes(body, MAX_BODY).await.map_err(|_| {
                Box::new(super::matrix::matrix_error(
                    StatusCode::PAYLOAD_TOO_LARGE,
                    "M_TOO_LARGE",
                    "invalid_input",
                    "请求体太大，无法判断邀请数量",
                ))
            })?;
            empty_invites = serde_json::from_slice::<serde_json::Value>(&bytes)
                .ok()
                .and_then(|v| {
                    v.get("invite")
                        .map(|list| list.as_array().is_some_and(|a| a.is_empty()))
                })
                .unwrap_or(false);
            parts.headers.remove(hyper::header::TRANSFER_ENCODING);
            parts.headers.insert(
                hyper::header::CONTENT_LENGTH,
                HeaderValue::from(bytes.len()),
            );
            Request::from_parts(parts, axum::body::Body::from(bytes))
        } else {
            req
        };
        // 空邀请表（建空房间）不算一次邀请；读不懂的按一次算，不猜。
        if empty_invites {
            return Ok(req);
        }
        let now = (self.now)();
        let decision = {
            let mut state = self.state.lock().expect("people limiter lock");
            state.decide(&self.limits, person, kind, txn.as_deref(), now)
        };
        match decision {
            Some(retry_after_ms) => {
                tracing::info!(person, ?kind, retry_after_ms, "本机的人发得太快，先挡住");
                Err(Box::new(rate_limited(retry_after_ms)))
            }
            None => Ok(req),
        }
    }
}

impl State {
    /// 放行时返回 `None`，被挡住时返回要等多久。
    fn decide(
        &mut self,
        limits: &PeopleLimits,
        person: &str,
        kind: Kind,
        txn: Option<&str>,
        now: u64,
    ) -> Option<u64> {
        if now.saturating_sub(self.last_sweep) >= SWEEP_EVERY.as_millis() as u64 {
            self.last_sweep = now;
            self.buckets
                .retain(|_, bucket| now.saturating_sub(bucket.last) < 60_000);
        }
        let per_minute = match kind {
            Kind::Invite => limits.invites_per_minute,
            Kind::Message => limits.messages_per_minute,
        };
        let capacity = per_minute as f64;
        let bucket = self
            .buckets
            .entry((person.to_owned(), kind))
            .or_insert_with(|| Bucket {
                tokens: capacity,
                last: now,
                txns: Vec::new(),
            });
        let elapsed = now.saturating_sub(bucket.last) as f64;
        bucket.tokens = (bucket.tokens + elapsed * per_minute as f64 / 60_000.0).min(capacity);
        bucket.last = now;
        if let Some(txn) = txn
            && bucket.txns.iter().any(|known| known == txn)
        {
            return None;
        }
        if bucket.tokens < 1.0 {
            let ms_per_token = 60_000.0 / per_minute as f64;
            return Some(((1.0 - bucket.tokens) * ms_per_token).ceil() as u64);
        }
        bucket.tokens -= 1.0;
        if let Some(txn) = txn {
            if bucket.txns.len() >= 64 {
                bucket.txns.remove(0);
            }
            bucket.txns.push(txn.to_owned());
        }
        None
    }
}

/// 数得上的请求：(种类, 去重用的 txn)。
fn classify(method: &Method, path: &str) -> Option<(Kind, Option<String>)> {
    let rest = path.strip_prefix("/_matrix/client/")?;
    let mut segments = rest.split('/');
    match segments.next() {
        Some(version) if !version.is_empty() => {}
        _ => return None,
    };
    match (method, segments.next()?) {
        (&Method::POST, "createRoom") => Some((Kind::Invite, None)),
        (&Method::POST, "rooms") => {
            let room = segments.next()?;
            if room.is_empty() || segments.next()? != "invite" {
                return None;
            }
            Some((Kind::Invite, None))
        }
        (&Method::PUT, "rooms") => {
            let room = segments.next()?;
            if room.is_empty() || segments.next()? != "send" {
                return None;
            }
            let kind = String::from_utf8(percent_decode(segments.next()?)).ok()?;
            if !MESSAGE_TYPES.contains(&kind.as_str()) {
                return None;
            }
            let txn = segments.next()?;
            if txn.is_empty() || segments.next().is_some() {
                return None;
            }
            Some((Kind::Message, Some(txn.to_owned())))
        }
        _ => None,
    }
}

fn rate_limited(retry_after_ms: u64) -> Response {
    let mut response = (
        StatusCode::TOO_MANY_REQUESTS,
        axum::Json(json!({
            "errcode": "M_LIMIT_EXCEEDED",
            "error": "发得太快，稍等再发",
            "retry_after_ms": retry_after_ms,
        })),
    )
        .into_response();
    response.headers_mut().insert(
        pages::ERROR_HEADER,
        HeaderValue::from_static("rate_limited"),
    );
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(method: Method, path: &str) -> Request {
        Request::builder()
            .method(method)
            .uri(format!("http://x{path}"))
            .body(axum::body::Body::from(r#"{"invite":["@a:b"]}"#))
            .unwrap()
    }

    #[test]
    fn counts_invites_messages_and_skips_other_types() {
        assert!(matches!(
            classify(&Method::POST, "/_matrix/client/v3/createRoom"),
            Some((Kind::Invite, _))
        ));
        assert!(matches!(
            classify(&Method::POST, "/_matrix/client/v3/rooms/!r/invite"),
            Some((Kind::Invite, _))
        ));
        assert!(matches!(
            classify(
                &Method::PUT,
                "/_matrix/client/v3/rooms/!r/send/m.room.encrypted/t1"
            ),
            Some((Kind::Message, Some(_)))
        ));
        assert!(
            classify(
                &Method::PUT,
                "/_matrix/client/v3/rooms/!r/send/m.reaction/t1"
            )
            .is_none()
        );
        assert!(classify(&Method::GET, "/_matrix/client/v3/createRoom").is_none());
    }

    #[tokio::test]
    async fn a_person_is_limited_per_minute_and_txns_do_not_double_count() {
        let limiter = PeopleLimiter::with_clock(
            PeopleLimits {
                invites_per_minute: 2,
                messages_per_minute: 2,
            },
            || 0,
        );
        assert!(
            limiter
                .check(req(Method::POST, "/_matrix/client/v3/createRoom"), "owner")
                .await
                .is_ok()
        );
        assert!(
            limiter
                .check(req(Method::POST, "/_matrix/client/v3/createRoom"), "owner")
                .await
                .is_ok()
        );
        let refused = limiter
            .check(req(Method::POST, "/_matrix/client/v3/createRoom"), "owner")
            .await;
        assert!(refused.is_err());
        // 别人是另一份。
        assert!(
            limiter
                .check(req(Method::POST, "/_matrix/client/v3/createRoom"), "mia")
                .await
                .is_ok()
        );
    }

    #[tokio::test]
    async fn an_empty_invite_list_does_not_count() {
        let limiter = PeopleLimiter::with_clock(PeopleLimits::default(), || 0);
        for _ in 0..100 {
            let request = Request::builder()
                .method(Method::POST)
                .uri("http://x/_matrix/client/v3/createRoom")
                .body(axum::body::Body::from(r#"{"invite":[]}"#))
                .unwrap();
            assert!(limiter.check(request, "owner").await.is_ok());
        }
    }
}
