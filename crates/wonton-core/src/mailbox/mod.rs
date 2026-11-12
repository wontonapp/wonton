//! Mailpit 收信、按插件来源限频并投递进每个收件人的 Wonton 私聊（ADR-0056）。

mod api;
mod component;
mod content;
mod delivery;
mod progress;

pub use component::Component;

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use tokio::task::JoinHandle;

use crate::gateway::Gateway;
use crate::http::HttpClient;

const RECOVERY_INTERVAL: Duration = Duration::from_secs(5);

/// 连不上时的重试退避：首失败 1 秒，逐次翻倍，封顶 5 秒。Mailpit 容器起来是秒级的事（启动时
/// DNS 名常常还没注册），不值得按分钟等。
const RETRY_BACKOFF: Duration = Duration::from_secs(1);
const RETRY_BACKOFF_MAX: Duration = RECOVERY_INTERVAL;

fn backoff(failures: u32) -> Duration {
    RETRY_BACKOFF
        .checked_mul(1 << failures.saturating_sub(1).min(3))
        .unwrap_or(RETRY_BACKOFF_MAX)
        .min(RETRY_BACKOFF_MAX)
}

pub struct Worker(JoinHandle<()>);

impl Drop for Worker {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// 订阅先于补漏，避免列信和建立订阅之间丢事件；聊天失败留在 Mailpit，定期重扫补投。
pub fn start(
    authority: &str,
    password: &str,
    http: HttpClient,
    gateway: Arc<Gateway>,
    state_dir: &Path,
) -> anyhow::Result<Worker> {
    let api = api::Api::new(authority, password, http)?;
    let mut delivery = delivery::Delivery::new(api, gateway, state_dir)?;
    Ok(Worker(tokio::spawn(async move {
        let mut failures = 0u32;
        let mut warned = false;
        loop {
            match delivery.api.subscribe().await {
                Ok(mut events) => {
                    failures = 0;
                    warned = false;
                    delivery.scan().await;
                    let mut recovery = tokio::time::interval(RECOVERY_INTERVAL);
                    recovery.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
                    recovery.tick().await;
                    let connected_at = std::time::Instant::now();
                    loop {
                        tokio::select! {
                            event = events.next() => match event {
                                Ok(Some(id)) => delivery.attempt(&id).await,
                                Ok(None) => break,
                                Err(error) => {
                                    tracing::debug!(%error, "收信事件连接已断开，立即重连补漏");
                                    break;
                                }
                            },
                            _ = recovery.tick() => delivery.scan().await,
                        }
                    }
                    // Mailpit 收不到 pong 会不带 close frame 直接关 TCP，重启也是；这是常态，
                    // 立即重连并靠订阅后的扫描补漏，不当作错误，也不为此等一个恢复间隔。
                    // 连上就断的抖动按失败退避，别对着一个还没就绪的端点打转。
                    if connected_at.elapsed() < RETRY_BACKOFF {
                        tokio::time::sleep(RETRY_BACKOFF).await;
                    }
                }
                Err(error) => {
                    failures += 1;
                    if error.is_transport() {
                        // 起动时 Mailpit 容器常常还没就绪（DNS 名还没注册）：warn 一次，之后每次重试 debug。
                        if warned {
                            tracing::debug!(%error, "Mailpit 还没连上，继续重试");
                        } else {
                            warned = true;
                            tracing::warn!(%error, "Mailpit 还没连上，稍后自动重试");
                        }
                    } else {
                        tracing::error!(%error, "连接 Mailpit 失败，未处理的信留在收件箱");
                    }
                    delivery.scan().await;
                    tokio::time::sleep(backoff(failures)).await;
                }
            }
        }
    })))
}

#[cfg(test)]
mod attachment_tests;
#[cfg(test)]
mod docker_tests;
#[cfg(test)]
mod event_support;
#[cfg(test)]
mod test_support;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod worker_tests;
