//! 假扮的家里服务器：隧道的对面是假核心。

use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;

use axum::Router;
use iroh::endpoint::presets;
use iroh::{Endpoint, EndpointId, SecretKey};
use tokio::net::TcpListener;
use wonton_tunnel::{Allowlist, TunnelServer};

use super::router::fake_core;
use super::state::CoreState;

pub struct FakeHome {
    pub core: Arc<CoreState>,
    pub allowed: Allowlist,
    pub(super) server: TunnelServer,
}

impl FakeHome {
    pub async fn start(allowed: Vec<EndpointId>) -> Self {
        let core = Arc::new(CoreState::default());
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let upstream = listener.local_addr().unwrap();
        let router = Router::new().fallback(fake_core).with_state(core.clone());
        tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        let endpoint = Endpoint::builder(presets::Minimal)
            .secret_key(SecretKey::generate())
            .clear_ip_transports()
            .bind_addr((Ipv4Addr::LOCALHOST, 0))
            .unwrap()
            .bind()
            .await
            .unwrap();
        let allowed = Allowlist::new(allowed);
        let server = TunnelServer::spawn(endpoint, allowed.clone(), upstream);
        Self {
            core,
            allowed,
            server,
        }
    }

    pub fn ticket(&self) -> String {
        ticket(
            self.server.endpoint().id(),
            self.server.endpoint().bound_sockets()[0],
        )
    }
}

pub fn ticket(id: EndpointId, addr: SocketAddr) -> String {
    format!(
        r#"{{"server": {{"endpoint_id": "{id}", "direct_addrs": ["{addr}"]}},
            "base_domain": "wonton.localhost"}}"#
    )
}
