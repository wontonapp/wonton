//! 连上之后向核心要一次它现在的地址，存进连接信息：服务器换了 IP，下一次建连第一个包还是直连，不用重新配对（ADR-0032）。

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use iroh::{EndpointId, RelayUrl};
use serde::Deserialize;
use url::Url;
use wonton_protocol::ProxySettings;

use super::HomeLink;
use super::ticket::Ticket;
use crate::chat::proxy_url;

/// 字段同核心的 `GET /_wonton/api/tunnel`。
#[derive(Deserialize)]
struct Reported {
    endpoint_id: EndpointId,
    #[serde(default)]
    relay_urls: Vec<RelayUrl>,
    #[serde(default)]
    direct_addrs: Vec<SocketAddr>,
}

const TIMEOUT: Duration = Duration::from_secs(15);

impl HomeLink {
    /// 失败不影响连接，下次连上再取。
    pub(super) async fn refresh_addresses(self: Arc<Self>) {
        let Some(text) = self.lock_ticket().clone() else {
            return;
        };
        let (Ok(ticket), Some(proxy)) = (Ticket::parse(&text), self.view().proxy) else {
            return;
        };
        match fetch(&ticket.core_url(), &proxy).await {
            Ok(reported) if reported.endpoint_id == ticket.server_id() => {
                self.remember(&text, &ticket, reported).await;
            }
            Ok(reported) => tracing::warn!(
                reported = %reported.endpoint_id.fmt_short(),
                "核心报告的身份和连接信息不一样，不更新地址"
            ),
            Err(message) => tracing::info!("取服务器的地址失败：{message}"),
        }
    }

    async fn remember(&self, text: &str, ticket: &Ticket, reported: Reported) {
        let Some(updated) = ticket.with_reported(reported.relay_urls, &reported.direct_addrs)
        else {
            return;
        };
        let new_text = match serde_json::to_string(&updated) {
            Ok(new_text) => new_text,
            Err(e) => {
                tracing::warn!("连接信息写不成 JSON：{e}");
                return;
            }
        };
        {
            let mut current = self.lock_ticket();
            // 取地址的时候换了服务器。
            if current.as_deref() != Some(text) {
                return;
            }
            if let Err(e) = self.storage.save_ticket(&new_text) {
                tracing::warn!("保存服务器的新地址失败：{e:?}");
                return;
            }
            *current = Some(new_text);
        }
        tracing::info!(direct_addrs = ?updated.direct_addrs(), "记下服务器的新地址");
        if let Some((_, client)) = self.current_link().await {
            client.set_server_addr(updated.server_addr());
        }
    }
}

async fn fetch(core_url: &Url, proxy: &ProxySettings) -> Result<Reported, String> {
    let url = core_url
        .join("/_wonton/api/tunnel")
        .expect("固定的路径总能拼上");
    let client = reqwest::Client::builder()
        .proxy(reqwest::Proxy::all(proxy_url(proxy)).map_err(|e| e.to_string())?)
        .timeout(TIMEOUT)
        .build()
        .map_err(|e| e.to_string())?;
    let response = client.get(url).send().await.map_err(|e| e.to_string())?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!("核心回了 {status}"));
    }
    let bytes = response.bytes().await.map_err(|e| e.to_string())?;
    serde_json::from_slice(&bytes).map_err(|e| e.to_string())
}
