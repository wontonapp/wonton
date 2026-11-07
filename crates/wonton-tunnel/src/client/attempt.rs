use std::{future::Future, sync::Arc, time::Duration};

use iroh::endpoint::Connection;
use tokio::{sync::watch, task::AbortHandle};

use super::{
    Inner,
    suspend::{Stamp, elapsed, now},
};
use crate::TunnelError;

/// 一次恢复可能先后唤醒计时任务、请求和壳的前台事件；共享刚完成的检查，避免再次换连接。
const REVALIDATE_REUSE: Duration = Duration::from_secs(1);

#[derive(Debug, Clone)]
struct Completed {
    result: Result<Connection, TunnelError>,
    at: Stamp,
}

/// 结果留在 watch 里；任务被取消时发送端丢弃，等的人重新取一次。
#[derive(Debug)]
pub(super) struct Attempt {
    result: watch::Receiver<Option<Completed>>,
    pub(super) task: AbortHandle,
    /// 还在退避或握手：换网时可以取消。确认现有连接的任务不能取消，否则会丢掉连接。
    pub(super) cancellable: bool,
    pub(super) revalidating: bool,
}

impl Attempt {
    pub(super) fn spawn(
        cancellable: bool,
        revalidating: bool,
        inner: &Arc<Inner>,
        work: impl Future<Output = Result<Connection, TunnelError>> + Send + 'static,
    ) -> Arc<Self> {
        let (sender, result) = watch::channel(None);
        let clock = inner.clock.clone();
        let inner = Arc::downgrade(inner);
        let task = tokio::spawn(async move {
            let result = work.await;
            let confirmed = result.as_ref().ok().map(Connection::stable_id);
            sender.send_replace(Some(Completed {
                result,
                at: now(clock.as_deref()),
            }));
            if revalidating
                && let Some(conn_id) = confirmed
                && let Some(inner) = inner.upgrade()
            {
                inner.confirm(conn_id);
            }
        })
        .abort_handle();
        Arc::new(Self {
            result,
            task,
            cancellable,
            revalidating,
        })
    }

    pub(super) fn outcome(&self) -> Option<Result<Connection, TunnelError>> {
        self.result
            .borrow()
            .as_ref()
            .map(|done| done.result.clone())
    }

    pub(super) fn recently_revalidated(&self, at: Stamp) -> bool {
        self.revalidating
            && self
                .result
                .borrow()
                .as_ref()
                .is_some_and(|done| elapsed(done.at, at) < REVALIDATE_REUSE)
    }

    /// 被取消时返回 `None`。
    pub(super) async fn finished(&self) -> Option<Result<Connection, TunnelError>> {
        let mut result = self.result.clone();
        result
            .wait_for(Option::is_some)
            .await
            .ok()?
            .as_ref()
            .map(|done| done.result.clone())
    }
}
