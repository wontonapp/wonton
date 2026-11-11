use super::{SharePeer, Sharing};
use crate::gateway::Gateway;
use crate::plugin::PluginManager;
use axum::extract::Request;
use std::{sync::Arc, time::Duration};
use tokio::sync::watch;
use wonton_protocol::GatewayRefusal;

/// The last guard clone stops its monitor; detached upgrades retain their own clone.
#[derive(Clone)]
pub(in crate::gateway) struct AccessGuard(Arc<GuardInner>);
struct GuardInner {
    cancelled: watch::Receiver<bool>,
    task: tokio::task::AbortHandle,
    permits: Vec<crate::shares::GrantPermit>,
}
impl Drop for GuardInner {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl AccessGuard {
    pub(in crate::gateway) async fn cancelled(&self) {
        let mut rx = self.0.cancelled.clone();
        if *rx.borrow_and_update() {
            return;
        }
        while rx.changed().await.is_ok() {
            if *rx.borrow_and_update() {
                return;
            }
        }
    }

    pub(super) fn allows(&self, access: &crate::shares::Access) -> bool {
        !*self.0.cancelled.borrow()
            && access
                .permits
                .iter()
                .any(|permit| self.0.permits.contains(permit))
    }

    pub(super) fn permits(&self) -> &[crate::shares::GrantPermit] {
        &self.0.permits
    }

    fn new(
        sharing: Arc<Sharing>,
        plugins: Arc<PluginManager>,
        peer: SharePeer,
        id: String,
    ) -> Result<Self, GatewayRefusal> {
        // Subscribe before spawning/checking, so a change between admission and the task is visible.
        let mut friends = sharing.friends.subscribe();
        let mut shares = sharing.shares.subscribe();
        let mut lifecycle = plugins.changes();
        let initial_access = sharing.authorize(&peer, &id, &plugins)?;
        let initial = initial_access.permits;
        let original = initial.clone();
        let (cancelled, rx) = watch::channel(false);
        let task = tokio::spawn(async move {
            loop {
                friends.borrow_and_update();
                shares.borrow_and_update();
                lifecycle.borrow_and_update();
                let checked_at = sharing.now();
                let access = match sharing.authorize(&peer, &id, &plugins) {
                    Ok(access) => access,
                    Err(_) => {
                        cancelled.send_replace(true);
                        return;
                    }
                };
                if !access.permits.iter().any(|permit| initial.contains(permit)) {
                    cancelled.send_replace(true);
                    return;
                }
                let deadline = async {
                    // A later/unbounded grant must not postpone expiry of an original permit.
                    let expiry = sharing.shares.next_deadline(checked_at);
                    match expiry {
                        Some(at) => {
                            tokio::time::sleep(Duration::from_millis(
                                at.saturating_sub(sharing.now()),
                            ))
                            .await
                        }
                        None => std::future::pending().await,
                    }
                };
                tokio::select! {
                    _ = deadline => {},
                    result = friends.changed() => if result.is_err() { break; },
                    result = shares.changed() => if result.is_err() { break; },
                    result = lifecycle.changed() => if result.is_err() { break; },
                }
            }
            cancelled.send_replace(true);
        });
        Ok(Self(Arc::new(GuardInner {
            cancelled: rx,
            task: task.abort_handle(),
            permits: original,
        })))
    }
}

/// Every plugin served on a keep-alive stream remains bound until that stream closes.
#[derive(Clone)]
pub(in crate::gateway) struct StreamAccess(
    watch::Sender<std::collections::BTreeMap<String, AccessGuard>>,
);
impl StreamAccess {
    pub(super) fn new() -> Self {
        Self(watch::channel(Default::default()).0)
    }
    fn get(&self, id: &str) -> Option<AccessGuard> {
        self.0.borrow().get(id).cloned()
    }
    fn add(&self, id: String, guard: AccessGuard) -> AccessGuard {
        let mut selected = guard;
        self.0.send_if_modified(|guards| {
            if let Some(existing) = guards.get(&id) {
                selected = existing.clone();
                false
            } else {
                guards.insert(id, selected.clone());
                true
            }
        });
        selected
    }
    pub(super) async fn cancelled(&self) {
        use futures_util::{
            FutureExt,
            stream::{FuturesUnordered, StreamExt},
        };
        let mut registered = self.0.subscribe();
        let mut known = std::collections::BTreeSet::new();
        let mut pending = FuturesUnordered::new();
        loop {
            {
                let guards = registered.borrow_and_update();
                for (id, guard) in guards.iter() {
                    if known.insert(id.clone()) {
                        let guard = guard.clone();
                        pending.push(async move { guard.cancelled().await }.boxed());
                    }
                }
            }
            tokio::select! {
                _ = pending.next(), if !pending.is_empty() => return,
                result = registered.changed() => if result.is_err() { return; },
            }
        }
    }
}

pub(in crate::gateway) fn guard_request(
    gateway: &Gateway,
    req: &mut Request,
    plugin_id: &str,
) -> Result<(), GatewayRefusal> {
    let peer = req
        .extensions()
        .get::<SharePeer>()
        .cloned()
        .ok_or(GatewayRefusal::ShareRequired)?;
    let sharing = gateway
        .sharing
        .as_ref()
        .ok_or(GatewayRefusal::ShareRequired)?;
    let access = sharing.authorize(&peer, plugin_id, &gateway.plugins)?;
    let stream = req.extensions().get::<StreamAccess>();
    let guard = match stream.and_then(|stream| stream.get(plugin_id)) {
        Some(guard) => guard,
        None => {
            let guard = AccessGuard::new(
                sharing.clone(),
                gateway.plugins.clone(),
                peer,
                plugin_id.to_owned(),
            )?;
            match stream {
                Some(stream) => stream.add(plugin_id.to_owned(), guard),
                None => guard,
            }
        }
    };
    if !guard.allows(&access) {
        return Err(GatewayRefusal::ShareRevoked);
    }
    req.extensions_mut().insert(guard);
    Ok(())
}

pub(in crate::gateway) fn guard_body(
    body: hyper::body::Incoming,
    guard: AccessGuard,
) -> axum::body::Body {
    use http_body_util::BodyExt;
    let frames = futures_util::stream::unfold(
        (body, guard, false),
        |(mut body, guard, stopped)| async move {
            if stopped {
                return None;
            }
            let next = tokio::select! {
                biased;
                _ = guard.cancelled() => Some(Err(std::io::Error::new(
                    std::io::ErrorKind::ConnectionAborted, "share access revoked",
                ))),
                frame = body.frame() => frame.map(|frame| frame.map_err(std::io::Error::other)),
            };
            next.map(|frame| {
                let stopped = frame.is_err();
                (frame, (body, guard, stopped))
            })
        },
    );
    axum::body::Body::new(http_body_util::StreamBody::new(frames))
}
