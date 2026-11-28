//! 回退任务的句柄被丢掉，回退照样把第二档加进来。

use std::time::Duration;

use iroh::test_utils::run_relay_server;
use iroh::{RelayUrl, SecretKey, Watcher, tls::CaTlsConfig};
use wonton_tunnel::relays::{RelayTable, relay_map, spawn_tier_fallback};

use super::relay_endpoint;
use crate::support::init_tracing;

#[tokio::test]
async fn dropping_the_fallback_handle_does_not_stop_the_fallback() -> anyhow::Result<()> {
    init_tracing();
    // 第一档没有人听，第二档是健康中继；句柄丢掉以后任务还得活着，超时后把第二档加进来。
    let (_map, healthy_url, healthy) = run_relay_server().await?;
    let dead: RelayUrl = "https://127.0.0.1:1/".parse().unwrap();
    let table = RelayTable::parse(&format!("{dead};{healthy_url}")).unwrap();
    let endpoint = relay_endpoint(SecretKey::generate(), relay_map(&table.views(), 1)).await;
    let fallback = spawn_tier_fallback(
        endpoint.clone(),
        table,
        Duration::from_millis(300),
        Some(CaTlsConfig::insecure_skip_verify()),
    );
    drop(fallback);
    let connected = tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            if endpoint
                .home_relay_status()
                .get()
                .iter()
                .any(|status| status.url() == &healthy_url && status.is_connected())
            {
                return;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await;
    endpoint.close().await;
    healthy.shutdown().await?;
    connected.expect("句柄 drop 后回退任务被 abort，第二档一直没加进来");
    Ok(())
}
