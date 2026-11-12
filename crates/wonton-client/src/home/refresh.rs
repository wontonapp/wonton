//! 连上之后向核心要它现在的地址，存进连接信息：服务器换了 IP，下一次建连第一个包还是直连，不用重新配对（ADR-0032）。

use std::net::SocketAddr;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Weak};
use std::time::Duration;

use futures_util::StreamExt;
use iroh::endpoint::PathEventStream;
use iroh::{Endpoint, EndpointId, TransportAddr};
use serde::Deserialize;
use url::Url;
use wonton_protocol::ProxySettings;
use wonton_tunnel::relays::Relay;
use wonton_tunnel::{TunnelClient, TunnelStatus};

use super::HomeLink;
use super::ticket::Ticket;
use crate::chat::proxy_url;

/// 字段同核心的 `GET /_wonton/api/tunnel`。
#[derive(Deserialize)]
struct Reported {
    endpoint_id: EndpointId,
    #[serde(default)]
    relays: Vec<Relay>,
    #[serde(default)]
    direct_addrs: Vec<SocketAddr>,
}

const TIMEOUT: Duration = Duration::from_secs(15);

/// 取失败后的退避。一次 GET 很便宜、单次限时 [`TIMEOUT`]，机器忙或者刚换网时失败一两次是常事；这几次加起来
/// 三十一秒，和 iroh 的空闲超时（30 秒）差不多，换网后几秒内就能补上。
const RETRY_DELAYS: [Duration; 5] = [
    Duration::from_secs(1),
    Duration::from_secs(2),
    Duration::from_secs(4),
    Duration::from_secs(8),
    Duration::from_secs(16),
];
/// 退避用完还没取到就按这个间隔一直取，直到取到或这条连接断开：再快也只是白刷，30 秒内补上足够下一次建连。
const RETRY_INTERVAL: Duration = Duration::from_secs(30);

/// 一次取地址的结果。
enum Step {
    /// 取到了，地址已经用上。
    Done,
    /// 这次没取到，按退避再来。
    Retry,
    /// 没东西可取，或者连接已经不在了：等下次连上。
    Stop,
}

impl HomeLink {
    /// 连上之后取核心现在的地址。失败按 [`RETRY_DELAYS`] 退避，之后每 [`RETRY_INTERVAL`] 一次，直到取到或这条连接
    /// 断开。失败期间 [`HomeLink::addresses_stale`] 为真，取到就清掉。断开不影响连接本身，下次连上重新开始。
    ///
    /// 同一时刻只跑一个：连上又断又连上时 [`HomeLink::spawn_address_refresh`] 会取消旧的。
    pub(super) async fn refresh_addresses(self: Arc<Self>) {
        let mut delays = RETRY_DELAYS.into_iter();
        loop {
            match self.refresh_addresses_once().await {
                Step::Done | Step::Stop => return,
                Step::Retry => {
                    self.set_addresses_stale(true);
                    tokio::time::sleep(delays.next().unwrap_or(RETRY_INTERVAL)).await;
                    // 退避期间连接断了：这次连接不再取，重连后的新任务从头开始。
                    if !self.connected().await {
                        return;
                    }
                }
            }
        }
    }

    async fn refresh_addresses_once(&self) -> Step {
        let Some(text) = self.lock_ticket().clone() else {
            return Step::Stop;
        };
        let (Ok(ticket), Some(proxy)) = (Ticket::parse(&text), self.view().proxy) else {
            return Step::Stop;
        };
        let started = std::time::Instant::now();
        match fetch(&ticket.core_url(), &proxy).await {
            Ok(reported) if reported.endpoint_id == ticket.server_id() => {
                // 存不下去也算没取到：磁盘抖一下的话，下一次可能就好了。
                if self.remember(&text, &ticket, reported).await {
                    self.set_addresses_stale(false);
                    Step::Done
                } else {
                    self.set_addresses_stale(true);
                    Step::Retry
                }
            }
            // 核心报的不是存下的那台服务器：存着的不算数，和取失败一样再取。
            Ok(reported) => {
                self.set_addresses_stale(true);
                tracing::warn!(
                    reported = %reported.endpoint_id.fmt_short(),
                    elapsed = ?started.elapsed(),
                    "reported identity doesn't match the saved connection info; keeping the old addresses"
                );
                Step::Retry
            }
            Err(message) => {
                self.set_addresses_stale(true);
                tracing::info!(elapsed = ?started.elapsed(), "could not fetch the server's addresses: {message}");
                Step::Retry
            }
        }
    }

    /// 本次连接选中的直连路径排最前。返回连接信息是否还是取地址时那份（是的话新地址已经用上）。
    async fn remember(&self, text: &str, ticket: &Ticket, reported: Reported) -> bool {
        let Some((_, client)) = self.current_link().await else {
            return false;
        };
        let Some(path) = client.path() else {
            return false;
        };
        let connected = match path.remote {
            TransportAddr::Ip(addr) => Some(addr),
            _ => None,
        };
        let Some(updated) =
            ticket.with_reported(reported.relays, &reported.direct_addrs, connected)
        else {
            return self.is_current_ticket(text);
        };
        let new_text = match serde_json::to_string(&updated) {
            Ok(new_text) => new_text,
            Err(e) => {
                tracing::warn!("could not serialize the connection info: {e}");
                return false;
            }
        };
        {
            let mut current = self.lock_ticket();
            // 取地址的时候换了服务器。
            if current.as_deref() != Some(text) {
                return false;
            }
            if let Err(e) = self.storage.save_ticket(&new_text) {
                tracing::warn!("could not save the server's new addresses: {e:?}");
                return false;
            }
            *current = Some(new_text);
        }
        tracing::info!(direct_addrs = ?updated.direct_addrs(), "saved the server's new addresses");
        client.set_server_addr(updated.server_addr());
        true
    }

    /// 存着的连接信息还是不是 `text`。
    fn is_current_ticket(&self, text: &str) -> bool {
        self.lock_ticket().as_deref() == Some(text)
    }

    /// 服务端现在的地址取到了没有。只有取地址的任务改它，变了就重算一次，壳才看得到。
    pub(super) fn set_addresses_stale(&self, stale: bool) {
        if self.addresses_stale.swap(stale, Ordering::Relaxed) != stale {
            self.recount();
        }
    }

    /// 当前的连接还连着（取地址的退避用它判断这次连接还算不算数）。
    pub(super) async fn connected(&self) -> bool {
        self.current_link()
            .await
            .is_some_and(|(_, client)| client.status() == TunnelStatus::Connected)
    }

    /// 开始取服务端现在的地址。上一次的还跑着就取消掉。
    pub(super) fn spawn_address_refresh(self: &Arc<Self>) {
        let mut slot = self
            .address_refresh
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(old) = slot.take() {
            old.abort();
        }
        *slot = Some(self.runtime.spawn(self.clone().refresh_addresses()));
    }

    pub(super) fn stop_address_refresh(&self) {
        let task = self
            .address_refresh
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take();
        if let Some(task) = task {
            task.abort();
        }
    }
}
/// 把隧道客户端的状态变化写进 ViewModel，断开期间定时重新算（断开的时长和中继的情况不发事件）。只持有弱引用，核心释放后
/// 自动结束。
pub(super) async fn follow_connection(
    home: Weak<HomeLink>,
    endpoint: Endpoint,
    client: TunnelClient,
) {
    let Some(mut recount) = home.upgrade().map(|home| home.recount.subscribe()) else {
        return;
    };
    let mut status = client.watch_status();
    let mut was_connected = false;
    let mut paths: Option<PathEventStream> = None;
    let mut selected = None;
    loop {
        let current = status.borrow_and_update().clone();
        let Some(link) = home.upgrade() else {
            return;
        };
        let connected = current == TunnelStatus::Connected;
        if connected && !was_connected {
            paths = client.path_events();
        }
        let path = client.path().map(|path| path.remote);
        let changed_to_direct = path != selected && path.as_ref().is_some_and(|path| path.is_ip());
        selected = path;
        let tick = link.publish(&current, &endpoint, &client);
        if connected && (!was_connected || changed_to_direct) {
            link.spawn_address_refresh();
        } else if !connected {
            link.stop_address_refresh();
            paths = None;
            selected = None;
        }
        was_connected = connected;
        drop(link);
        tokio::select! {
            event = async { paths.as_mut()?.next().await }, if paths.is_some() => {
                if event.is_none() {
                    paths = None;
                }
            }
            changed = status.changed() => {
                if changed.is_err() {
                    return;
                }
            }
            changed = recount.changed() => {
                if changed.is_err() {
                    return;
                }
            }
            () = tokio::time::sleep(tick.unwrap_or_default()), if tick.is_some() => {}
        }
    }
}

async fn fetch(core_url: &Url, proxy: &ProxySettings) -> Result<Reported, String> {
    let url = core_url
        .join("/_wonton/api/tunnel")
        .expect("fixed tunnel path always joins");
    let client = reqwest::Client::builder()
        .proxy(reqwest::Proxy::all(proxy_url(proxy)).map_err(|e| e.to_string())?)
        .timeout(TIMEOUT)
        .build()
        .map_err(|e| e.to_string())?;
    let response = client.get(url).send().await.map_err(|e| e.to_string())?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!("home server returned {status}"));
    }
    let bytes = response.bytes().await.map_err(|e| e.to_string())?;
    serde_json::from_slice(&bytes).map_err(|e| e.to_string())
}
