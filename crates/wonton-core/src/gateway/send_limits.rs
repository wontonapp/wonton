//! Bot 发消息的频率和跳数（ADR-0045 的“频率和跳数”）。只数
//! `PUT /_matrix/client/{版本}/rooms/{房间}/send/{类型}/{txn}`，类型是 `m.room.message`、`m.room.encrypted`、
//! `m.sticker`；状态事件、回执、输入状态、to-device 都不算。设备和 Bot 两个入口分开：Bot 的发送按（Bot 的
//! 访问令牌哈希，房间）限频、按房间数跳数；设备（人）的发送只把所在房间的跳数清零。
//!
//! - **频率**：令牌桶，每个（令牌哈希，房间）每分钟最多 [`SendLimits::per_minute`] 条，桶容量
//!   [`SendLimits::burst`]。超了回 429 `M_LIMIT_EXCEEDED` 带 `retry_after_ms`，SDK 自己会等。编辑也算。
//! - **跳数**：数人上一次说话之后，不同 Bot 之间的来回：一条 Bot 消息的发送者和这个房间上一条 Bot 消息的发送者
//!   不同，才算一跳。同一个 Bot 连着发（进度、分段回复）不算；编辑（`m.relates_to.rel_type` 是 `m.replace`）
//!   既不算一跳，也不改“上一条是谁发的”。设备发一条清零；房间闲置 [`SendLimits::idle_reset`] 没有消息也清零，
//!   条目一并清掉，内存不无限涨；主人在“已停下”卡片上选“继续”也清零（[`SendLimiter::resume`]）。到了
//!   [`SendLimits::hop_limit`] 跳，再来一跳回 403，`errcode` 是 [`HOP_LIMIT_ERRCODE`]、`x-wonton-error: hop_limit`，
//!   SDK 不重试；还是上一条那个 Bot 接着说照样放行。
//! - **同一个 txn 重发**（SDK 重试）不重复计数：数过的 txn 记在桶里，重发直接放行，由上游按 txn 去重。
//! - 计数只在内存里，核心重启从头算（ADR-0045 允许近似）。
//!
//! 只为认编辑读一遍请求体：`m.relates_to` 在加密消息里也是明文（Matrix 规范要求，matrix-sdk 照做），读到的原始
//! 字节原样装回。Bot 的用户 ID 用 `whoami` 取，按令牌哈希缓存；取不到不挡消息，按令牌哈希认 Bot。

pub(super) mod hop_card;
#[cfg(test)]
mod hop_limit_tests;

use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::body::{Body, to_bytes};
use axum::extract::Request;
use axum::response::{IntoResponse, Response};
use hyper::header::{AUTHORIZATION, HeaderName, HeaderValue};
use hyper::{Method, StatusCode};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use wonton_protocol::{GATEWAY_REFUSAL_HEADER, GatewayRefusal, HOP_LIMIT_ERRCODE};

use super::matrix::{matrix_error, percent_decode};
use super::pages;
use crate::matrix::Matrix;

/// 数得上的发送事件类型：只有这三种算 Bot 在说话。
const COUNTED_TYPES: [&str; 3] = ["m.room.message", "m.room.encrypted", "m.sticker"];

/// 认编辑时读请求体的上限：Matrix 的事件最大 64 KiB，留一倍余量。
const MAX_BODY: usize = 128 * 1024;

/// 每个（令牌哈希，房间）记多少条发过的 txn：SDK 的重试都在几秒内，64 条绰绰有余。
const TXN_RING: usize = 64;

/// 多久扫一遍把没动的条目清掉；条目闲置 [`SendLimits::idle_reset`] 后，用到时也会就地清零。
const SWEEP_EVERY: Duration = Duration::from_secs(60);

/// 网关管 Bot 发消息的数字（ADR-0045）。默认值按 ADR，核心的环境变量能改（见 `config.rs` 的 `send_limits`）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SendLimits {
    /// 每个 Bot 在每个房间每分钟最多发几条（`WONTON_BOT_MSG_PER_MINUTE`）。
    pub per_minute: u32,
    /// 令牌桶容量：可以突发多少条（`WONTON_BOT_MSG_BURST`）。
    pub burst: u32,
    /// 每个房间里，人上一次说话之后不同 Bot 之间最多来回几跳（`WONTON_BOT_MSG_HOP_LIMIT`）。
    pub hop_limit: u32,
    /// 房间多久没有消息就把跳数清零（`WONTON_BOT_MSG_IDLE_RESET_SECS`）。
    pub idle_reset: Duration,
}

impl Default for SendLimits {
    fn default() -> Self {
        Self {
            per_minute: 20,
            burst: 10,
            hop_limit: 20,
            idle_reset: Duration::from_secs(60 * 60),
        }
    }
}

/// 跳数到顶：某个房间里几个 Bot 来回说得太多，被网关停下。一个回合只出一次，拿它发“继续”卡片。
#[derive(Clone, Debug)]
pub(crate) struct HopLimitReached {
    pub room_id: String,
    /// 这个回合里在说话的 Bot（含被停下的那个），排序去重；认不出用户 ID 的按令牌哈希记，不在这里。
    pub bot_user_ids: Vec<String>,
}

/// Bot 的发送被挡住：给 Bot 的响应，跳数到顶的那一次带上事件。
pub(crate) struct Refused {
    pub response: Response,
    pub stopped: Option<HopLimitReached>,
}

impl Refused {
    fn plain(response: Response) -> Self {
        Self {
            response,
            stopped: None,
        }
    }
}

/// Unix 毫秒。测试注入假的时钟，不许 sleep。
pub(crate) type Clock = Arc<dyn Fn() -> u64 + Send + Sync>;

pub(crate) fn system_clock() -> Clock {
    Arc::new(|| {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("系统时间在 Unix 纪元之前")
            .as_millis() as u64
    })
}

/// 数得上的发送（设备和 Bot 都用）：返回百分号解码后的房间 ID 和请求里的 txn。
fn counted_send(method: &Method, path: &str) -> Option<(String, String)> {
    if *method != Method::PUT {
        return None;
    }
    let rest = path.strip_prefix("/_matrix/client/")?;
    let mut segments = rest.split('/');
    match segments.next() {
        Some(version) if !version.is_empty() => {}
        _ => return None,
    }
    if segments.next()? != "rooms" {
        return None;
    }
    let room = room_id(segments.next()?)?;
    if segments.next()? != "send" {
        return None;
    }
    let kind = String::from_utf8(percent_decode(segments.next()?)).ok()?;
    if !COUNTED_TYPES.contains(&kind.as_str()) {
        return None;
    }
    let txn = segments.next()?;
    if txn.is_empty() || segments.next().is_some() {
        return None;
    }
    Some((room, txn.to_owned()))
}

/// 设备入口用：是不是数得上的发送，返回房间 ID。
pub(super) fn counted_room(method: &Method, path: &str) -> Option<String> {
    counted_send(method, path).map(|(room, _)| room)
}

/// 百分号解码一个路径参数；空的或解出来不是 UTF-8 的都不是合法的房间 ID。
fn room_id(segment: &str) -> Option<String> {
    if segment.is_empty() {
        return None;
    }
    String::from_utf8(percent_decode(segment)).ok()
}

fn bearer(req: &Request) -> Option<String> {
    let token = req.headers().get(AUTHORIZATION)?.to_str().ok()?;
    token
        .strip_prefix("Bearer ")
        .filter(|token| !token.is_empty())
        .map(str::to_owned)
}

pub(crate) fn token_hash(token: &str) -> [u8; 32] {
    Sha256::digest(token.as_bytes()).into()
}

/// 一次频率和跳数的判断。
enum Decision {
    /// 放行（含同一个 txn 的重发，那条不算新的）。
    Pass,
    RateLimited {
        retry_after_ms: u64,
    },
    /// 跳数到顶；`announce` 只在一个回合的第一次为真，事件就发一次。
    HopLimited {
        announce: bool,
        bot_user_ids: Vec<String>,
    },
}

struct Bucket {
    tokens: f64,
    last: u64,
    txns: TxnRing,
}

#[derive(Default)]
struct TxnRing {
    queue: VecDeque<String>,
    seen: HashSet<String>,
}

impl TxnRing {
    fn has(&self, txn: &str) -> bool {
        self.seen.contains(txn)
    }

    fn record(&mut self, txn: &str) {
        if self.queue.len() >= TXN_RING
            && let Some(oldest) = self.queue.pop_front()
        {
            self.seen.remove(&oldest);
        }
        if self.seen.insert(txn.to_owned()) {
            self.queue.push_back(txn.to_owned());
        }
    }
}

/// 一条数得上的、认得出 Bot 的 Bot 发送（第 8 块挂叫回标签用）：哪个房间、哪个 Bot、
/// 话题根（主时间线是没有话题的 `None`）。编辑不带——它不是新的一句话。
pub(crate) struct SentMessage {
    pub(super) room: String,
    pub(super) bot_user: String,
    pub(super) thread: Option<String>,
}

/// 一个房间的跳数：人上一次说话之后不同 Bot 之间来回了几跳，上一条（不算编辑的）Bot 消息是谁发的，和这个
/// 回合里在说话的 Bot。
#[derive(Default)]
struct RoomRun {
    hop: u32,
    last_message: u64,
    fired: bool,
    last_bot: Option<String>,
    bots: BTreeSet<String>,
}

impl RoomRun {
    /// 人说了话，或房间闲置够了：从头来。
    fn reset(&mut self, now: u64) {
        *self = Self {
            last_message: now,
            ..Self::default()
        };
    }
}

struct CachedUser {
    user_id: String,
    last_used: u64,
}

#[derive(Default)]
struct State {
    buckets: HashMap<([u8; 32], String), Bucket>,
    rooms: HashMap<String, RoomRun>,
    users: HashMap<[u8; 32], CachedUser>,
    last_sweep: u64,
}

impl State {
    fn sweep(&mut self, limits: &SendLimits, now: u64) {
        if now.saturating_sub(self.last_sweep) < SWEEP_EVERY.as_millis() as u64 {
            return;
        }
        self.last_sweep = now;
        let idle = limits.idle_reset.as_millis() as u64;
        self.buckets
            .retain(|_, bucket| now.saturating_sub(bucket.last) < idle);
        self.rooms
            .retain(|_, run| now.saturating_sub(run.last_message) < idle);
        self.users
            .retain(|_, user| now.saturating_sub(user.last_used) < idle);
    }

    fn decide(&mut self, limits: &SendLimits, send: &BotSend<'_>, now: u64) -> Decision {
        let BotSend {
            hash,
            room,
            txn,
            bot,
            user,
            edit,
        } = *send;
        self.sweep(limits, now);
        let idle_ms = limits.idle_reset.as_millis() as u64;
        let bucket = self
            .buckets
            .entry((hash, room.to_owned()))
            .or_insert_with(|| Bucket {
                tokens: limits.burst as f64,
                last: now,
                txns: TxnRing::default(),
            });
        refill(bucket, limits, now);
        // SDK 重试的是数过的那条：上游按 txn 去重，网关也不再计数。
        if bucket.txns.has(txn) {
            return Decision::Pass;
        }
        // 跳数先判：被拒的那条不占频率的令牌，人说一句之后立刻能发。
        let run = self.rooms.entry(room.to_owned()).or_default();
        if now.saturating_sub(run.last_message) >= idle_ms {
            run.reset(now);
        }
        // 换了一个 Bot 说话才算一跳；同一个 Bot 接着说、任何 Bot 改自己的消息都不算。
        let turn = !edit && run.last_bot.as_deref().is_some_and(|last| last != bot);
        if turn && run.hop >= limits.hop_limit {
            let announce = !run.fired;
            run.fired = true;
            return Decision::HopLimited {
                announce,
                bot_user_ids: run.bots.iter().cloned().collect(),
            };
        }
        if bucket.tokens < 1.0 {
            let ms_per_token = 60_000.0 / limits.per_minute as f64;
            return Decision::RateLimited {
                retry_after_ms: ((1.0 - bucket.tokens) * ms_per_token).ceil() as u64,
            };
        }
        bucket.tokens -= 1.0;
        bucket.last = now;
        if turn {
            run.hop += 1;
        }
        if !edit {
            run.last_bot = Some(bot.to_owned());
        }
        run.last_message = now;
        if let Some(user) = user {
            run.bots.insert(user.to_owned());
        }
        bucket.txns.record(txn);
        Decision::Pass
    }
}

/// 按闲置的时间补回令牌，补满到桶容量。
fn refill(bucket: &mut Bucket, limits: &SendLimits, now: u64) {
    let elapsed = now.saturating_sub(bucket.last) as f64;
    let rate = limits.per_minute as f64 / 60_000.0;
    bucket.tokens = (bucket.tokens + elapsed * rate).min(limits.burst as f64);
    bucket.last = now;
}

/// 一条要判断的 Bot 发送。
#[derive(Clone, Copy)]
struct BotSend<'a> {
    hash: [u8; 32],
    room: &'a str,
    txn: &'a str,
    /// 认 Bot 用的键：用户 ID，认不出时是令牌哈希。
    bot: &'a str,
    user: Option<&'a str>,
    edit: bool,
}

/// Bot 发消息的频率和跳数。状态全在内存里。
pub struct SendLimiter {
    limits: SendLimits,
    now: Clock,
    state: Mutex<State>,
}

impl SendLimiter {
    pub fn new(limits: SendLimits) -> Self {
        Self::with_clock(limits, system_clock())
    }

    pub(crate) fn with_clock(limits: SendLimits, now: Clock) -> Self {
        Self {
            limits,
            now,
            state: Mutex::new(State::default()),
        }
    }

    pub(crate) fn hop_limit(&self) -> u32 {
        self.limits.hop_limit
    }

    /// 设备（人）在这个房间发了一条消息：跳数清零，重新计时。只对数得上的发送调用。
    pub(crate) fn device_sent(&self, room: &str) {
        self.reset(room);
    }

    /// 主人在“已停下”卡片上选了“继续”：同人说话一样清零。
    pub(crate) fn resume(&self, room: &str) {
        self.reset(room);
    }

    fn reset(&self, room: &str) {
        let now = (self.now)();
        let mut state = self.state.lock().expect("发送限制的锁");
        state.sweep(&self.limits, now);
        state.rooms.entry(room.to_owned()).or_default().reset(now);
    }

    /// Bot 入口的发送：数得上的请求先过频率和跳数，放行的原样返回（请求体读过一遍，原样装回）。
    /// 数得上的、认得出 Bot 的非编辑发送带回 [`SentMessage`]，给叫回标签用（第 8 块）。
    pub(crate) async fn check_bot_send(
        &self,
        req: Request,
        matrix: Option<&Matrix>,
    ) -> Result<(Request, Option<SentMessage>), Box<Refused>> {
        let Some((room, txn)) = counted_send(req.method(), req.uri().path()) else {
            return Ok((req, None));
        };
        // 没带访问令牌的请求不归这里管：上游照样会 401。
        let Some(token) = bearer(&req) else {
            return Ok((req, None));
        };
        let hash = token_hash(&token);
        let user = self.user_id(&token, hash, matrix).await;
        let (req, edit, thread) = read_edit(req).await?;
        let now = (self.now)();
        let bot = user.clone().unwrap_or_else(|| hex(&hash));
        let send = BotSend {
            hash,
            room: &room,
            txn: &txn,
            bot: &bot,
            user: user.as_deref(),
            edit,
        };
        let decision = {
            let mut state = self.state.lock().expect("发送限制的锁");
            state.decide(&self.limits, &send, now)
        };
        // 挂叫回标签的素材：认得出 Bot 的新消息（编辑不是新的一句话，也不带自己的话题根）。
        let sent = if let (Decision::Pass, false, Some(bot_user)) = (&decision, edit, &user) {
            Some(SentMessage {
                room: room.clone(),
                bot_user: bot_user.clone(),
                thread,
            })
        } else {
            None
        };
        match decision {
            Decision::Pass => Ok((req, sent)),
            Decision::RateLimited { retry_after_ms } => {
                tracing::info!(room = %room, retry_after_ms, "Bot 发消息太快，先挡住");
                Err(Box::new(Refused::plain(rate_limited(retry_after_ms))))
            }
            Decision::HopLimited {
                announce,
                bot_user_ids,
            } => {
                let event = HopLimitReached {
                    room_id: room,
                    bot_user_ids: with_the_stopped_bot(bot_user_ids, user),
                };
                tracing::warn!(
                    room = %event.room_id,
                    bots = ?event.bot_user_ids,
                    "几个 Bot 在一个房间里来回说得太多，已停下"
                );
                Err(Box::new(Refused {
                    response: hop_limited(),
                    stopped: announce.then_some(event),
                }))
            }
        }
    }

    /// 令牌是谁：先查缓存，没有就问 Tuwunel 的 `whoami`。取不到不挡消息。
    pub(crate) async fn user_id(
        &self,
        token: &str,
        hash: [u8; 32],
        matrix: Option<&Matrix>,
    ) -> Option<String> {
        let matrix = matrix?;
        {
            let mut state = self.state.lock().expect("发送限制的锁");
            if let Some(cached) = state.users.get_mut(&hash) {
                cached.last_used = (self.now)();
                return Some(cached.user_id.clone());
            }
        }
        let user_id = matrix
            .whoami(token)
            .await
            .inspect_err(|error| tracing::warn!(error = %error, "辨认 Bot 的访问令牌失败"))
            .ok()?;
        self.state.lock().expect("发送限制的锁").users.insert(
            hash,
            CachedUser {
                user_id: user_id.clone(),
                last_used: (self.now)(),
            },
        );
        Some(user_id)
    }
}

/// 事件里的 Bot 名单：本回合在说话的，加上被停下的这个，排序去重。
fn with_the_stopped_bot(bot_user_ids: Vec<String>, user: Option<String>) -> Vec<String> {
    let mut ids: BTreeSet<String> = bot_user_ids.into_iter().collect();
    if let Some(user) = user {
        ids.insert(user);
    }
    ids.into_iter().collect()
}

fn hex(hash: &[u8; 32]) -> String {
    hash.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// 读出请求体看是不是编辑（`m.relates_to.rel_type` 是 `m.replace`）和话题根（`rel_type` 是
/// `m.thread` 时的 `event_id`；加密消息的 `m.relates_to` 在密文外，两种都读得到），再把原始
/// 字节装回去。读不出 JSON 的不算编辑、没有话题，交给上游判断。
async fn read_edit(req: Request) -> Result<(Request, bool, Option<String>), Box<Refused>> {
    let (parts, body) = req.into_parts();
    let Ok(bytes) = to_bytes(body, MAX_BODY).await else {
        return Err(Box::new(Refused::plain(matrix_error(
            StatusCode::PAYLOAD_TOO_LARGE,
            "M_TOO_LARGE",
            "invalid_input",
            "消息太大",
        ))));
    };
    let relates = serde_json::from_slice::<Value>(&bytes)
        .ok()
        .and_then(|content| content.pointer("/m.relates_to").cloned());
    let rel_type = relates
        .as_ref()
        .and_then(|relates| relates.get("rel_type"))
        .and_then(Value::as_str);
    let edit = rel_type == Some("m.replace");
    let thread = (rel_type == Some("m.thread"))
        .then(|| {
            relates
                .as_ref()
                .and_then(|relates| relates.get("event_id"))
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .flatten();
    Ok((Request::from_parts(parts, Body::from(bytes)), edit, thread))
}

/// 跳数到顶的拒绝：403，`errcode` 是 [`HOP_LIMIT_ERRCODE`]，SDK 不重试，又能从错误里认出类别（matrix-sdk 读
/// 不到响应头，`M_FORBIDDEN` 的其余字段它也丢掉）；响应头照样带 [`GATEWAY_REFUSAL_HEADER`]。
fn hop_limited() -> Response {
    let mut response = (
        StatusCode::FORBIDDEN,
        axum::Json(json!({
            "errcode": HOP_LIMIT_ERRCODE,
            "error": "人上一次在这个房间说话之后，几个 Bot 来回说得太多了，等人说一句或主人选“继续”",
        })),
    )
        .into_response();
    response.headers_mut().insert(
        HeaderName::from_static(GATEWAY_REFUSAL_HEADER),
        HeaderValue::from_static(GatewayRefusal::HopLimit.as_str()),
    );
    response
}

fn rate_limited(retry_after_ms: u64) -> Response {
    let mut response = (
        StatusCode::TOO_MANY_REQUESTS,
        axum::Json(json!({
            "errcode": "M_LIMIT_EXCEEDED",
            "error": "这个房间里消息发得太密，稍等再发",
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
