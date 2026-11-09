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
        loop {
            match delivery.api.subscribe().await {
                Ok(mut events) => {
                    delivery.scan().await;
                    let mut recovery = tokio::time::interval(RECOVERY_INTERVAL);
                    recovery.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
                    recovery.tick().await;
                    loop {
                        tokio::select! {
                            event = events.next() => match event {
                                Ok(Some(id)) => delivery.attempt(&id).await,
                                Ok(None) => break,
                                Err(error) => {
                                    tracing::error!(%error, "收信事件连接已断开，重连后补漏");
                                    break;
                                }
                            },
                            _ = recovery.tick() => delivery.scan().await,
                        }
                    }
                }
                Err(error) => {
                    tracing::error!(%error, "连接 Mailpit 失败，未处理的信留在收件箱");
                    delivery.scan().await;
                }
            }
            tokio::time::sleep(RECOVERY_INTERVAL).await;
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
