//! 确认现有连接是不是还活着：限时做一次往返，没回应就判死、换连接。
//!
//! 判死不能只看 ping 那一条流：机器忙的时候这一趟往返会被排过时限，连接却一直在给别的流送数据，换连接会让正在
//! 转发的请求跟着断（本地代理给用户回 502）。窗口里有新数据交到调用方手上就算活着，一个字节都没有才判死。
//!
//! 为什么不数“收到的包”（`udp_rx`）：链路一头断了的时候，对端照样把没确认的旧数据反复重传回来，`udp_rx` 每轮
//! 都在涨，判活会一直成立、拖到 QUIC 的 30 秒空闲超时。
//!
//! 也不数确认帧（`frame_rx.acks`）：确认会把窗口开始前就发出的旧包算成“窗口里的动静”，链路刚断、旧包还在路上时
//! 正好误判成活着（`stall::a_proven_slow_path_is_still_replaced_when_it_dies` 在 CI 上就是这么红的）。
//!
//! 也不只数收进来的 STREAM 帧（`frame_rx.stream`）：半通的链路（手机发出去的包全丢、对端发来的还能到）上，对端收不到
//! 确认，会把已经收到过的偏移一遍遍重传，每一遍都是 STREAM 帧；对端只要还有发送额度，也还在送新数据——帧和新数据都
//! 在涨，可我们的包一个也到不了，新请求一样过不去。
//!
//! 所以两件事都要有：窗口里有新数据交到调用方（对端在送）**并且**对端确认过我们发去的包（对端听得到我们）。
//! 前者单独会被半通链路满足，后者单独会被在途旧包的确认满足（`stall::a_proven_slow_path_is_still_replaced_when_it_dies`
//! 在 CI 上红过），两个一起才等于“这一趟还能用”。
//!
//! 判死之后先拨新连接、再关旧的：新请求立刻走新连接，旧连接上已经在跑的流还有 [`RETIRE_GRACE`] 收完。

use std::{
    sync::{Arc, atomic::Ordering},
    time::{Duration, Instant},
};

use iroh::TransportAddr;
use iroh::endpoint::Connection;
use tracing::{debug, info};

use super::{
    Attempt, CLOSE_NORMAL, REVALIDATE_DEADLINE, TunnelClient, TunnelError, TunnelStatus, lock,
    revalidation_window, streams::StreamCounters, suspend, was_rejected,
};
use crate::error::from_connection_error;

/// 判死的连接交给还在跑的流这么久收尾，然后才关：正常的一个往返在忙的机器上也就一秒上下，留出它的两倍；
/// 长轮询和下载本来就会被调用方重来。
const RETIRE_GRACE: Duration = Duration::from_secs(2);

/// 一次限时确认的结论。
enum Verdict {
    /// ping 有了结果：通了，或者当场出错。
    Ping(std::result::Result<(), TunnelError>),
    /// ping 到点没回；带的是窗口后半段还能用的证据（新数据和确认都没有时是 `None`）。
    Silent(Option<((u64, u64), u64)>),
}

impl TunnelClient {
    /// 确认现有连接仍然可用。App 从挂起中恢复（回到前台）时调用。
    ///
    /// 挂起超过路径的空闲超时后，服务端会放弃这条连接，关闭帧却到不了挂起的设备：不检查的话，要等本机的
    /// 空闲超时（iroh 默认 30 秒）才发现，期间所有请求都挂着。这里限时做一次往返，没有回应就拨新连接、让旧连接
    /// 收完手上的流；窗口**后半段**里既有新数据又有对端的确认才算它活着。检查期间新开的流等检查结果，再走确认过的
    /// 连接或新连接。没有连接时等同于 [`ping`]。
    ///
    /// [`ping`]: Self::ping
    pub async fn revalidate(&self) -> Result<Duration, TunnelError> {
        if self.inner.closed.load(Ordering::SeqCst) {
            return Err(TunnelError::Closed);
        }
        let started = Instant::now();
        let attempt = self.start_revalidation();
        let conn = match attempt.finished().await {
            Some(result) => result?,
            None => return self.ping().await,
        };
        if let Some(reason) = conn.close_reason() {
            return Err(from_connection_error(&reason));
        }
        if !attempt.revalidating {
            self.ping_on(&conn).await?;
        }
        tracing::info!(
            reused = !attempt.revalidating,
            elapsed = ?started.elapsed(),
            "revalidate: kept the connection"
        );
        self.inner.confirm(conn.stable_id());
        Ok(started.elapsed())
    }

    pub(super) fn start_revalidation(&self) -> Arc<Attempt> {
        lock(&self.inner.pacing).reset();
        let mut current = lock(&self.inner.attempt);
        let mut live = None;
        if let Some(attempt) = current.as_ref() {
            tracing::info!(
                outcome = match attempt.outcome() {
                    None => "pending",
                    Some(Ok(conn)) if conn.close_reason().is_none() => "connected",
                    Some(Ok(_)) => "disconnected",
                    Some(Err(err)) => err.code(),
                },
                revalidating = attempt.revalidating,
                "revalidate: existing attempt"
            );
            match attempt.outcome() {
                None => return attempt.clone(),
                Some(Ok(conn)) if was_rejected(&conn) => return attempt.clone(),
                Some(Err(TunnelError::Unauthorized)) => return attempt.clone(),
                Some(result) => {
                    let usable = match &result {
                        Ok(conn) => conn.close_reason().is_none(),
                        Err(_) => true,
                    };
                    if usable
                        && attempt.recently_revalidated(suspend::now(self.inner.clock.as_deref()))
                    {
                        return attempt.clone();
                    }
                    live = result.ok().filter(|conn| conn.close_reason().is_none());
                }
            }
        }
        let attempt = if let Some(conn) = live {
            Attempt::spawn(
                false,
                true,
                &self.inner,
                self.clone().confirm_or_replace(conn),
            )
        } else {
            let client = self.clone();
            Attempt::spawn(true, true, &self.inner, async move {
                let conn = client.inner.clone().dial().await?;
                if let Err(err) = client.ping_on(&conn).await {
                    client
                        .inner
                        .set_status(TunnelStatus::Disconnected(err.clone()));
                    return Err(err);
                }
                Ok(conn)
            })
        };
        *current = Some(attempt.clone());
        attempt
    }

    /// 若现在 [`revalidate`]，会用多长的确认窗口。给测试把「慢路径的证据是否改变了决策」钉成确定性
    /// 断言用：窗口只取决于连接、路径和探测证据，与机器负载无关。
    ///
    /// [`revalidate`]: Self::revalidate
    #[doc(hidden)]
    pub fn next_revalidate_window(&self) -> Duration {
        self.inner
            .live_connection()
            .map_or(REVALIDATE_DEADLINE, |conn| self.revalidate_deadline(&conn))
    }

    /// 限时确认隧道真的通：有连接时在它上面做一次往返，没有回应就换连接，连接状态和路径随之更新；没有连接时
    /// 等同于 [`ping`]。
    ///
    /// QUIC 自己的判断不够：心跳和确认这些小包还能往返时，连接算活着，请求却可能过不去（#154）。反过来，机器一忙
    /// 这一趟往返也可能排过时限，所以判死要看别的证据：窗口里既有新数据交给调用方、对端又确认过我们发去的包才算
    /// 活着，缺一样就换。和 [`revalidate`] 不同，检查期间新开的流照常走现有连接，所以可以定时调用，不拖慢请求。
    ///
    /// [`ping`]: Self::ping
    /// [`revalidate`]: Self::revalidate
    pub async fn probe(&self, deadline: Duration) -> Result<Duration, TunnelError> {
        let Some(conn) = self.inner.live_connection() else {
            return self.ping().await;
        };
        let path = CheckedPath::selected(&conn);
        let started = Instant::now();
        let delivered = self.inner.streams.received();
        let acked = ack_frames(&conn);
        let received = conn.stats().udp_rx.datagrams;
        match tokio::time::timeout(deadline, self.ping_on(&conn)).await {
            Ok(Ok(())) => {
                let elapsed = started.elapsed();
                if self.inner.is_current(conn.stable_id()) && path == CheckedPath::selected(&conn) {
                    self.record_checked_path(path, elapsed);
                }
                Ok(elapsed)
            }
            Ok(Err(err)) => Err(err),
            Err(_) => {
                // 我这一趟没回来，可窗口里既有新数据交到调用方、对端又确认过我们发去的包：对端忙不是路断了，
                // 换连接会让正在转发的请求跟着断。
                if let Some(proof) = still_usable(&self.inner.streams, &conn, delivered, acked) {
                    let received = received_since(&conn, received).unwrap_or(0);
                    debug!(
                        ?deadline,
                        bytes = proof.0,
                        acked = proof.1,
                        received,
                        "现有连接在窗口里还能用，不换"
                    );
                    let elapsed = started.elapsed();
                    if self.inner.is_current(conn.stable_id())
                        && path == CheckedPath::selected(&conn)
                    {
                        self.record_checked_path(path, elapsed);
                    }
                    return Ok(elapsed);
                }
                info!(?deadline, "现有连接限时内没有回应，断开");
                let err = TunnelError::transport("连接限时内没有回应");
                // 先写状态再换建连：订阅状态的人读路径时，这条连接已经不算了。
                self.inner
                    .mark(conn.stable_id(), TunnelStatus::Disconnected(err.clone()));
                if self.inner.is_current(conn.stable_id()) {
                    let client = self.clone();
                    let attempt = Attempt::spawn(true, false, &self.inner, async move {
                        client.dial_replacement(Some(conn)).await
                    });
                    *lock(&self.inner.attempt) = Some(attempt);
                }
                Err(err)
            }
        }
    }

    fn record_checked_path(&self, path: Option<CheckedPath>, elapsed: Duration) {
        let mut proof = lock(&self.inner.slow_path);
        let Some(path) = path.filter(|_| elapsed >= REVALIDATE_DEADLINE) else {
            *proof = None;
            return;
        };
        match proof.as_mut() {
            Some(previous) if previous.path == path => {
                previous.samples = previous.samples.saturating_add(1).min(2);
            }
            _ => *proof = Some(SlowPath { path, samples: 1 }),
        }
    }

    fn revalidate_deadline(&self, conn: &Connection) -> Duration {
        let Some(path) = CheckedPath::selected(conn) else {
            return REVALIDATE_DEADLINE;
        };
        let slow = lock(&self.inner.slow_path);
        let proven_slow = slow
            .as_ref()
            .is_some_and(|proof| proof.path == path && proof.samples == 2);
        revalidation_window(path.relay, proven_slow)
    }

    /// 限时在 `conn` 上做一次往返；没回应就换新连接。
    ///
    /// 到点时还看一条证据：**窗口后半段**里既有新数据又有确认才算它活着——对端忙不是路断了。只看整个窗口
    /// 有没有涨是不够的：服务端换了 endpoint（同一身份、同一端口，手机上收不到任何通知）时，断掉那一刻在路上的
    /// 几个包会落在窗口前段，旧连接于是被当成“还活着”留下来，和空闲超时一样要等三十秒。后半段还涨，才说明
    /// 对端**现在**还在送、还在确认。
    async fn confirm_or_replace(self, conn: Connection) -> Result<Connection, TunnelError> {
        let deadline = self.revalidate_deadline(&conn);
        let probed = Instant::now();
        tracing::info!(
            ?deadline,
            path = ?CheckedPath::selected(&conn),
            "confirm: probing"
        );
        // 借用（`self`、`conn`）到这一块结束为止：结论先算出来，再决定拿谁往下走。
        let verdict = {
            let ping = self.ping_on(&conn);
            tokio::pin!(ping);
            tokio::select! {
                result = &mut ping => Verdict::Ping(result),
                () = tokio::time::sleep(deadline / 2) => {
                    // 窗口过半，从这一拍起算“还有没有动静”。
                    let delivered = self.inner.streams.received();
                    let acked = ack_frames(&conn);
                    let received = conn.stats().udp_rx.datagrams;
                    let left = deadline.saturating_sub(probed.elapsed());
                    match tokio::time::timeout(left, &mut ping).await {
                        Ok(result) => Verdict::Ping(result),
                        Err(_) => Verdict::Silent(
                            still_usable(&self.inner.streams, &conn, delivered, acked)
                                .map(|proof| (proof, received_since(&conn, received).unwrap_or(0))),
                        ),
                    }
                }
            }
        };
        match verdict {
            Verdict::Ping(Ok(())) => {
                tracing::info!(elapsed = ?probed.elapsed(), "confirm: ping ok");
                Ok(conn)
            }
            Verdict::Ping(Err(err)) => {
                info!(code = err.code(), "现有连接不可用，重新建连");
                self.dial_replacement(Some(conn)).await
            }
            Verdict::Silent(Some((proof, received))) => {
                tracing::info!(
                    ?deadline,
                    bytes = proof.0,
                    acked = proof.1,
                    received,
                    elapsed = ?probed.elapsed(),
                    "confirm: still sending and acking, keeping the connection"
                );
                Ok(conn)
            }
            Verdict::Silent(None) => {
                info!(?deadline, "现有连接窗口后半段没有回应，换新连接");
                self.dial_replacement(Some(conn)).await
            }
        }
    }

    /// 换掉一条判死的连接：先拨新的并做一次往返，旧连接这时才交给 [`retire`]，让还在跑的流收完。
    async fn dial_replacement(self, old: Option<Connection>) -> Result<Connection, TunnelError> {
        // 收尾从那一个判断开始算，不跟着拨号走：路完全不通时建连要挂满 12 秒，旧连接上的流不该陪着等。
        // 新请求走新连接由换掉的 attempt 保证，不需要等新连接建好才关旧的。
        if let Some(old) = old {
            retire(old);
        }
        let conn = self.inner.clone().dial().await?;
        if let Err(err) = self.ping_on(&conn).await {
            self.inner
                .set_status(TunnelStatus::Disconnected(err.clone()));
            return Err(err);
        }
        Ok(conn)
    }
}

#[derive(Debug, PartialEq, Eq)]
struct CheckedPath {
    conn_id: usize,
    remote: TransportAddr,
    relay: bool,
}

impl CheckedPath {
    fn selected(conn: &Connection) -> Option<Self> {
        let paths = conn.paths();
        let path = paths.iter().find(|path| path.is_selected())?;
        Some(Self {
            conn_id: conn.stable_id(),
            remote: path.remote_addr().clone(),
            relay: path.is_relay(),
        })
    }
}

#[derive(Debug)]
pub(super) struct SlowPath {
    path: CheckedPath,
    samples: u8,
}

/// 窗口里对端又发来了几个包，只用于日志。
fn received_since(conn: &Connection, before: u64) -> Option<u64> {
    let now = conn.stats().udp_rx.datagrams;
    (now > before).then(|| now - before)
}

/// 窗口里有多少字节交到了调用方手上；一个字节都没有时返回 `None`。
///
/// 只数新数据：对端重传已经收到过的偏移时，包、帧、确认都在涨，交给调用方的却不变。
fn delivered_since(counters: &StreamCounters, before: u64) -> Option<u64> {
    let now = counters.received();
    (now > before).then(|| now - before)
}

/// 窗口里对端确认了我们发去的几个包；一条确认都没有时返回 `None`。
fn acked_since(conn: &Connection, before: u64) -> Option<u64> {
    let now = ack_frames(conn);
    (now > before).then(|| now - before)
}

fn ack_frames(conn: &Connection) -> u64 {
    let stats = conn.stats();
    stats.frame_rx.acks + stats.frame_rx.path_acks
}

/// 这一趟还通的证据：窗口里既有新数据交给调用方（对端在送），对端又确认过我们发去的包（对端听得到我们）。
///
/// 两者缺一都不够：半通的链路能送来新数据却收不到我们的包；刚断的链路会有一批旧包的确认在窗口里才到。
fn still_usable(
    counters: &StreamCounters,
    conn: &Connection,
    delivered: u64,
    acked: u64,
) -> Option<(u64, u64)> {
    let bytes = delivered_since(counters, delivered)?;
    let acked = acked_since(conn, acked)?;
    Some((bytes, acked))
}

/// 判死的旧连接：新连接确认可用之后，让旧连接上的流还有 [`RETIRE_GRACE`] 收尾，再关。
///
/// 先拨后关：新请求立刻走新连接，旧连接上已经在跑的流不至于跟着断。本机自己关掉的连接，断开原因不算
/// 传输问题，状态由判死的一方写。
fn retire(conn: Connection) {
    tokio::spawn(async move {
        tokio::time::sleep(RETIRE_GRACE).await;
        conn.close(CLOSE_NORMAL, b"replaced");
    });
}
