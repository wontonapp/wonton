//! Bot 发消息的频率和跳数（ADR-0045 的“频率和跳数”），只看请求路径，不看内容（房间是加密的）。只数
//! `PUT /_matrix/client/{版本}/rooms/{房间}/send/{类型}/{txn}`，类型是 `m.room.message`、`m.room.encrypted`、
//! `m.sticker`；状态事件、回执、输入状态、to-device 都不算。设备和 Bot 两个入口分开：Bot 的发送按（Bot 的
//! 访问令牌哈希，房间）限频、按房间数跳数；设备（人）的发送只把所在房间的跳数清零。
//!
//! - **频率**：令牌桶，每个（令牌哈希，房间）每分钟最多 [`SendLimits::per_minute`] 条，桶容量
//!   [`SendLimits::burst`]。超了回 429 `M_LIMIT_EXCEEDED` 带 `retry_after_ms`，SDK 自己会等。
//! - **跳数**：每个房间记“人上一次在这里说话之后，Bot 连着发了几条”。设备发一条清零；房间闲置
//!   [`SendLimits::idle_reset`] 没有消息也清零，条目一并清掉，内存不无限涨。超过 [`SendLimits::hop_limit`] 条后
//!   回 403 `M_FORBIDDEN`（`x-wonton-error: hop_limit`），SDK 不重试。
//! - **同一个 txn 重发**（SDK 重试）不重复计数：数过的 txn 记在桶里，重发直接放行，由上游按 txn 去重。
//! - 计数只在内存里，核心重启从头算（ADR-0045 允许近似）。
//!
//! 跳数到顶时发一个 [`HopLimitReached`]（`tokio::sync::broadcast`），给后续发“继续”卡片的任务订阅；现在只记
//! 日志。Bot 的用户 ID 用 `whoami` 取，按令牌哈希缓存（`family-bots` 合并后可换成它的缓存）；取不到不挡消息，
//! 事件里少一个名字，限频和跳数照常按令牌哈希执行。

use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::extract::Request;
use axum::response::{IntoResponse, Response};
use hyper::header::{AUTHORIZATION, HeaderValue};
use hyper::{Method, StatusCode};
use serde_json::json;
use sha2::{Digest, Sha256};
use tokio::sync::broadcast;
use wonton_protocol::GatewayRefusal;

use super::matrix::{percent_decode, refusal};
use super::pages;
use crate::matrix::Matrix;

/// 数得上的发送事件类型：只有这三种算 Bot 在说话。
const COUNTED_TYPES: [&str; 3] = ["m.room.message", "m.room.encrypted", "m.sticker"];

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
    /// 每个房间里，人上一次说话之后 Bot 连着最多发几条（`WONTON_BOT_MSG_HOP_LIMIT`）。
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

/// 跳数到顶：某个房间里 Bot 连着发得太多，被网关停下。后续发“继续”卡片的任务订阅这个事件。
#[derive(Clone, Debug)]
pub(crate) struct HopLimitReached {
    pub room_id: String,
    /// 这个回合里在说话的 Bot（含被停下的那个）；令牌辨认不出来时缺一个名字。
    pub bot_user_ids: Vec<String>,
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

fn token_hash(token: &str) -> [u8; 32] {
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

/// 一个房间的跳数：人上一次说话之后 Bot 连着发的条数，和这个回合里在说话的 Bot（按令牌辨认出的用户 ID）。
#[derive(Default)]
struct RoomRun {
    hop: u32,
    last_message: u64,
    fired: bool,
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

    fn decide(
        &mut self,
        limits: &SendLimits,
        hash: [u8; 32],
        room: &str,
        txn: &str,
        now: u64,
        user: Option<&str>,
    ) -> Decision {
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
        if run.hop >= limits.hop_limit {
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
        run.hop += 1;
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

/// Bot 发消息的频率和跳数。状态全在内存里。
pub struct SendLimiter {
    limits: SendLimits,
    now: Clock,
    state: Mutex<State>,
    events: broadcast::Sender<HopLimitReached>,
}

impl SendLimiter {
    pub fn new(limits: SendLimits) -> Self {
        Self::with_clock(limits, system_clock())
    }

    pub(crate) fn with_clock(limits: SendLimits, now: Clock) -> Self {
        let (events, _) = broadcast::channel(16);
        Self {
            limits,
            now,
            state: Mutex::new(State::default()),
            events,
        }
    }

    /// 订阅跳数到顶的事件。发“继续”卡片的任务还没来，现在只有测试在收；它落地时去掉 `cfg` 就行。
    #[cfg(test)]
    pub(crate) fn subscribe(&self) -> broadcast::Receiver<HopLimitReached> {
        self.events.subscribe()
    }

    /// 设备（人）在这个房间发了一条消息：跳数清零，重新计时。只对数得上的发送调用。
    pub(crate) fn device_sent(&self, room: &str) {
        let now = (self.now)();
        let mut state = self.state.lock().expect("发送限制的锁");
        state.sweep(&self.limits, now);
        state.rooms.entry(room.to_owned()).or_default().reset(now);
    }

    /// Bot 入口的发送：数得上的请求先过频率和跳数，放行的原样返回（请求体动都没动）。
    pub(crate) async fn check_bot_send(
        &self,
        req: Request,
        matrix: Option<&Matrix>,
    ) -> Result<Request, Box<Response>> {
        let Some((room, txn)) = counted_send(req.method(), req.uri().path()) else {
            return Ok(req);
        };
        // 没带访问令牌的请求不归这里管：上游照样会 401。
        let Some(token) = bearer(&req) else {
            return Ok(req);
        };
        let hash = token_hash(&token);
        let now = (self.now)();
        let user = self.user_id(&token, hash, matrix).await;
        let decision = {
            let mut state = self.state.lock().expect("发送限制的锁");
            state.decide(&self.limits, hash, &room, &txn, now, user.as_deref())
        };
        match decision {
            Decision::Pass => Ok(req),
            Decision::RateLimited { retry_after_ms } => {
                tracing::info!(room = %room, retry_after_ms, "Bot 发消息太快，先挡住");
                Err(Box::new(rate_limited(retry_after_ms)))
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
                    "Bot 在一个房间里连着说得太多，已停下"
                );
                if announce {
                    let _ = self.events.send(event);
                }
                Err(Box::new(refusal(
                    GatewayRefusal::HopLimit,
                    "人上一次在这个房间说话之后，Bot 连着发得太多了，等人说一句再继续",
                )))
            }
        }
    }

    /// 令牌是谁：先查缓存，没有就问 Tuwunel 的 `whoami`。取不到不挡消息。
    async fn user_id(
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
