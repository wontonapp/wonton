//! 到一台服务器的连接：endpoint、隧道客户端和本地代理同生共死；换服务器、断开时整体收尾。

use iroh::Endpoint;
use tokio::task::JoinHandle;
use wonton_tunnel::{LocalProxy, TunnelClient};

/// 到一台服务器的连接：endpoint、隧道客户端和本地代理同生共死。
pub(super) struct Link {
    pub(super) endpoint: Endpoint,
    pub(super) client: TunnelClient,
    pub(super) proxy: LocalProxy,
    pub(super) registration: wonton_tunnel::sharing::ProxyRegistration,
    pub(super) watcher: JoinHandle<()>,
    pub(super) checker: JoinHandle<()>,
}

impl Link {
    pub(super) async fn close(self) {
        self.watcher.abort();
        self.checker.abort();
        drop(self.registration);
        self.proxy.shutdown().await;
        self.client.close();
        self.endpoint.close().await;
    }
}
