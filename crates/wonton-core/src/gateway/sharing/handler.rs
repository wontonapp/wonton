use super::{SharePeer, StreamAccess};
use crate::{
    gateway::Gateway,
    plugin::{languages::Languages, manifest::PluginId},
};
use axum::body::Body;
use hyper::{body::Incoming, server::conn::http1, service::service_fn};
use hyper_util::rt::TokioIo;
use iroh::endpoint::Connection;
use std::sync::Arc;
use tower::ServiceExt;
use wonton_protocol::{GatewayRefusal, MatrixOpenIdToken, SharedPlugins};
use wonton_tunnel::sharing::{
    AuthenticatedShares, BoxFuture, ShareHandler, ShareRefusal, ShareSession, ShareStream,
};

pub struct SharingHandler {
    gateway: Arc<Gateway>,
}
impl SharingHandler {
    pub fn new(gateway: Arc<Gateway>) -> Self {
        Self { gateway }
    }

    fn listing(&self, peer: &SharePeer) -> Result<SharedPlugins, ShareRefusal> {
        let sharing = self.gateway.sharing.as_ref().ok_or_else(required)?;
        sharing.person(peer).map_err(refusal)?;
        let plugins = sharing
            .shares
            .active_plugin_ids(&peer.user_id, sharing.now())
            .into_iter()
            .filter_map(|id| {
                sharing.authorize(peer, &id, &self.gateway.plugins).ok()?;
                let id = PluginId::parse(&id).ok()?;
                self.gateway
                    .plugins
                    .shared_plugin(&id, &Languages::default())
            })
            .collect();
        Ok(SharedPlugins {
            user_id: peer.user_id.clone(),
            plugins,
        })
    }
}
fn refusal(reason: GatewayRefusal) -> ShareRefusal {
    ShareRefusal {
        refusal: reason,
        identity: None,
    }
}
fn required() -> ShareRefusal {
    refusal(GatewayRefusal::ShareRequired)
}

impl ShareHandler for SharingHandler {
    fn authenticate(
        &self,
        token: MatrixOpenIdToken,
    ) -> BoxFuture<'_, Result<AuthenticatedShares, ShareRefusal>> {
        Box::pin(async move {
            let sharing = self.gateway.sharing.as_ref().ok_or_else(required)?;
            let user_id = sharing
                .identity
                .verify(&token)
                .await
                .map_err(ShareRefusal::identity)?;
            let friend = sharing.friends.get(&user_id).ok_or_else(required)?;
            let peer = SharePeer {
                user_id,
                generation: friend.access_generation,
            };
            let shares = self.listing(&peer)?;
            Ok(AuthenticatedShares {
                shares,
                generation: peer.generation,
            })
        })
    }

    fn list<'a>(
        &'a self,
        session: &'a ShareSession,
    ) -> BoxFuture<'a, Result<SharedPlugins, ShareRefusal>> {
        Box::pin(async move {
            self.listing(&SharePeer {
                user_id: session.user_id().to_owned(),
                generation: session.generation,
            })
        })
    }

    fn forward<'a>(
        &'a self,
        session: &'a ShareSession,
        _connection: Connection,
        stream: ShareStream,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            let peer = SharePeer {
                user_id: session.user_id().to_owned(),
                generation: session.generation,
            };
            serve_io(self.gateway.clone(), peer, stream).await;
        })
    }
}

pub(super) async fn serve_io<I>(gateway: Arc<Gateway>, peer: SharePeer, stream: I)
where
    I: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let tracker = StreamAccess::new();
    let session_gateway = gateway.clone();
    let session_peer = peer.clone();
    let cancellation = tracker.clone();
    let app = super::super::router(gateway);
    let service = service_fn(move |req: hyper::Request<Incoming>| {
        let mut req = req.map(Body::new);
        req.extensions_mut().insert(peer.clone());
        req.extensions_mut().insert(tracker.clone());
        app.clone().oneshot(req)
    });
    let connection = http1::Builder::new()
        .serve_connection(TokioIo::new(stream), service)
        .with_upgrades();
    tokio::select! {
        _ = connection => {},
        _ = session_revoked(session_gateway, session_peer) => {},
        _ = cancellation.cancelled() => {},
    }
}

async fn session_revoked(gateway: Arc<Gateway>, peer: SharePeer) {
    let Some(sharing) = &gateway.sharing else {
        return;
    };
    let mut changes = sharing.friends.subscribe();
    loop {
        changes.borrow_and_update();
        if sharing.person(&peer).is_err() {
            return;
        }
        if changes.changed().await.is_err() {
            return;
        }
    }
}
