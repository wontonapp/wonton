use std::sync::Arc;
use std::sync::atomic::Ordering;

use wonton_protocol::{HomeStatus, PluginProblem, StoreListing, StoreLoadState};

use super::{PluginsLink, SOURCE_TIMEOUT, lock};

#[derive(Default)]
pub(super) struct Request {
    pub(super) revision: u64,
    /// 建连期间收到的强制刷新，等连接可用后仍须带给服务器。
    pub(super) force: bool,
    #[cfg(feature = "test-hooks")]
    in_flight: std::sync::atomic::AtomicUsize,
}

impl PluginsLink {
    pub(crate) fn refresh_store(self: &Arc<Self>, force: bool) {
        if self.external {
            return;
        }
        self.store_wanted.store(true, Ordering::Relaxed);
        let mut request = lock(&self.store_request);
        request.revision += 1;
        request.force |= force;
        let home = self.home.view();
        // 还没配对就没有可连的服务器：先让壳引导去配对，配对后 follow_home 会自己再取。
        if home.status == HomeStatus::NotConfigured {
            self.view
                .send_modify(|view| view.store_load = StoreLoadState::NotPaired);
            return;
        }
        if home.status == HomeStatus::Connecting {
            self.view
                .send_modify(|view| view.store_load = StoreLoadState::WaitingForConnection);
            return;
        }
        let revision = request.revision;
        let force = std::mem::take(&mut request.force);
        let server = home.server_id;
        let generation = self.generation.load(Ordering::SeqCst);
        self.view
            .send_modify(|view| view.store_load = StoreLoadState::Loading);
        #[cfg(feature = "test-hooks")]
        request.in_flight.fetch_add(1, Ordering::SeqCst);
        drop(request);
        let link = self.clone();
        self.runtime.spawn(async move {
            link.home.retry_if_rejected().await;
            let result = link.fetch_store(force).await;
            // 和发起/换服务器同锁：晚到的内容和错误都不能覆盖后一次请求。
            let request = lock(&link.store_request);
            if request.revision == revision && link.is_current_home(generation, &server) {
                link.view.send_modify(|view| match result {
                    Ok(store) => {
                        view.store = Some(store);
                        view.store_load = StoreLoadState::Ready;
                        Self::derive(view);
                    }
                    Err(problem) => view.store_load = StoreLoadState::Failed { problem },
                });
            }
            #[cfg(feature = "test-hooks")]
            request.in_flight.fetch_sub(1, Ordering::SeqCst);
        });
    }

    #[cfg(feature = "test-hooks")]
    pub(crate) fn store_requests_in_flight(&self) -> usize {
        lock(&self.store_request).in_flight.load(Ordering::SeqCst)
    }

    async fn fetch_store(&self, force: bool) -> Result<StoreListing, PluginProblem> {
        let query: &[(&str, &str)] = if force { &[("refresh", "true")] } else { &[] };
        self.api()?.get(&["store"], query, SOURCE_TIMEOUT).await
    }
}
