//! 核心的出箱（ADR-0061 第 10 节）：长轮询 `GET /agent/outbox` 取要做的项，做完
//! `POST` 回报。`CancelTurn` 对会话发 `session/cancel`（同会话未决的权限请求一并回
//! `cancelled`，这是发取消一方的义务），回报结果；`Message` 是发消息那块（ADR-0061
//! 第 9 节）的项；不认识的项如实回报并跳过，不猜怎么执行。
//!
//! 取走的项不回报会留在核心的在途表里，所以每项都回报；回报失败只记日志，不重发。

use std::time::Duration;

use wonton_protocol::{
    AGENT_OUTBOX_WAIT_SECS, OutboxFailure, OutboxItem, OutboxKind, OutboxReceipt, OutboxResult,
};

use crate::bridge::Bridge;
use crate::bridge::chain::agent_endpoint;
use crate::error::{Detail, Error};

/// 长轮询最多挂 [`AGENT_OUTBOX_WAIT_SECS`] 秒，客户端超时给它留出余量。
const POLL_TIMEOUT: Duration = Duration::from_secs(AGENT_OUTBOX_WAIT_SECS + 10);
/// 核心连不上时等一下再挂下一次长轮询（和启动时等 Agent 就绪同一个节奏）。
const RETRY_AFTER: Duration = Duration::from_secs(1);

/// 出箱的长轮询循环：进程活着就一直在取。桥退出时任务随运行时结束。
pub(crate) fn spawn(bridge: Bridge) {
    tokio::spawn(run(bridge));
}

pub(crate) async fn run(bridge: Bridge) {
    let feed = match Feed::new(&bridge) {
        Ok(feed) => feed,
        Err(error) => {
            tracing::error!(error = %error, "出箱接口的地址配不出来，出箱停了");
            return;
        }
    };
    loop {
        match feed.poll().await {
            Ok(Some(item)) => handle(&bridge, &feed, item).await,
            Ok(None) => {}
            Err(error) => {
                tracing::warn!(error = %error, "取出箱失败，等一下再试");
                tokio::time::sleep(RETRY_AFTER).await;
            }
        }
    }
}

async fn handle(bridge: &Bridge, feed: &Feed, item: OutboxItem) {
    let result = match &item.kind {
        OutboxKind::CancelTurn { session, .. } => cancel_turn(bridge, session).await,
        OutboxKind::Message { .. } => {
            tracing::warn!(id = %item.id, "出箱的 Message 项还没实现，跳过");
            skipped()
        }
        OutboxKind::Other => {
            tracing::warn!(id = %item.id, "不认识的出箱项，跳过");
            skipped()
        }
    };
    let receipt = OutboxReceipt {
        id: item.id,
        result,
    };
    if let Err(error) = feed.receipt(&receipt).await {
        tracing::warn!(id = %receipt.id, error = %error, "出箱回报没送到");
    }
}

async fn cancel_turn(bridge: &Bridge, session: &str) -> OutboxResult {
    if !bridge.sessions.contains_session(session).await {
        tracing::warn!(session, "出箱要取消的会话不在这座桥上");
        return OutboxResult::Failed {
            category: OutboxFailure::Other,
        };
    }
    // 桥自己挂在确认接口上的长轮询也是未决的权限请求，一并断开。
    bridge.pending_asks.cancel_session(session);
    match bridge.acp.cancel(session).await {
        // 取消不是房间里的事件，`event_id` 按协议留空。
        Ok(()) => OutboxResult::Sent {
            event_id: String::new(),
        },
        Err(error) => {
            tracing::warn!(session, error = %error, "会话取消没发出去");
            OutboxResult::Failed {
                category: OutboxFailure::Other,
            }
        }
    }
}

fn skipped() -> OutboxResult {
    OutboxResult::Failed {
        category: OutboxFailure::Other,
    }
}

struct Feed {
    http: reqwest::Client,
    endpoint: url::Url,
}

impl Feed {
    fn new(bridge: &Bridge) -> Result<Self, Error> {
        Ok(Self {
            http: reqwest::Client::builder()
                .timeout(POLL_TIMEOUT)
                .build()
                .map_err(|error| {
                    Error::InvalidInput(Detail::msg_with(
                        "outbox-client",
                        [("error", error.to_string())],
                    ))
                })?,
            endpoint: agent_endpoint(&bridge.config.mcp_url, wonton_protocol::AGENT_OUTBOX_PATH)?,
        })
    }

    /// 取一项；没有（长轮询超时给 `null`）返回 `None`。
    async fn poll(&self) -> Result<Option<OutboxItem>, Error> {
        let response = self
            .http
            .get(self.endpoint.clone())
            .send()
            .await
            .map_err(unavailable)?;
        let status = response.status();
        let body = response.bytes().await.map_err(unavailable)?;
        if !status.is_success() {
            return Err(Error::Upstream(Detail::data(format!("HTTP {status}"))));
        }
        serde_json::from_slice(&body)
            .map_err(|error| Error::Upstream(Detail::data(error.to_string())))
    }

    async fn receipt(&self, receipt: &OutboxReceipt) -> Result<(), Error> {
        let response = self
            .http
            .post(self.endpoint.clone())
            .json(receipt)
            .send()
            .await
            .map_err(unavailable)?;
        let status = response.status();
        if !status.is_success() {
            return Err(Error::Upstream(Detail::data(format!("HTTP {status}"))));
        }
        Ok(())
    }
}

fn unavailable(error: reqwest::Error) -> Error {
    Error::TransportUnavailable(Detail::data(error.to_string()))
}
