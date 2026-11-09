//! 测试里的回环地址 endpoint。同一端口重建时端口可能还被旧 endpoint 占着，也可能刚空出来就被并行的
//! 测试的 `bind(0)` 抢走：直接反复 bind 到绑上为止，不先探一次空再绑，两步之间不会被抢。

use std::net::Ipv4Addr;
use std::time::Duration;

use iroh::endpoint::presets;
use iroh::{Endpoint, SecretKey};

/// 等端口空出来的总预算，同 `home::support::RESTART`。
const RESTART: Duration = Duration::from_secs(60);

pub(super) async fn loopback_endpoint(key: SecretKey, port: u16) -> Endpoint {
    let deadline = tokio::time::Instant::now() + RESTART;
    loop {
        let bound = Endpoint::builder(presets::Minimal)
            .secret_key(key.clone())
            .clear_ip_transports()
            .bind_addr((Ipv4Addr::LOCALHOST, port))
            .unwrap()
            .bind()
            .await;
        match bound {
            Ok(endpoint) => return endpoint,
            Err(error) if crate::home::support::port_in_use(&error) => {
                assert!(
                    tokio::time::Instant::now() < deadline,
                    "隧道的端口一直没有释放"
                );
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
            Err(error) => panic!("回环地址的端点起不来：{error}"),
        }
    }
}
