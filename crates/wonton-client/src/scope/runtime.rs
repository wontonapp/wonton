//! 一个作用域的 tokio 运行时。停下以后这个作用域的任务都结束，再交来的任务报“已经切到另一台服务器”。

use std::future::Future;
use std::sync::{Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::Duration;

use tokio::runtime::{Handle, Runtime};
use wonton_l10n::l10n;
use wonton_protocol::CoreError;

use crate::l10n::{CATALOG, language};

/// 停下时最多等旧任务这么久。
const STOP_TIMEOUT: Duration = Duration::from_secs(1);

pub(crate) struct ScopeRuntime(Mutex<Option<Runtime>>);

impl ScopeRuntime {
    pub(crate) fn new() -> Self {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("wonton-core")
            .enable_all()
            .build()
            .expect("could not create the tokio runtime");
        Self(Mutex::new(Some(runtime)))
    }

    pub(crate) fn handle(&self) -> Option<Handle> {
        self.lock().as_ref().map(|runtime| runtime.handle().clone())
    }

    /// 在这个运行时里执行：壳（Swift / Kotlin）驱动导出的 async 方法时，线程上没有 tokio 运行时，matrix-sdk 的
    /// 网络操作离不开它。
    pub(crate) async fn run<T: Send + 'static>(
        &self,
        task: impl Future<Output = Result<T, CoreError>> + Send + 'static,
    ) -> Result<T, CoreError> {
        let handle = self.handle().ok_or_else(switched)?;
        handle.spawn(task).await.map_err(|error| {
            if error.is_cancelled() {
                switched()
            } else {
                CoreError::Upstream {
                    reason: l10n!(
                        CATALOG,
                        language(),
                        "scope-task-crashed",
                        error = error.to_string()
                    ),
                }
            }
        })?
    }

    pub(crate) fn spawn(&self, task: impl Future<Output = ()> + Send + 'static) {
        if let Some(handle) = self.handle() {
            handle.spawn(task);
        }
    }

    /// 停下并等旧任务结束，最多一秒。
    pub(crate) fn stop(&self) {
        if let Some(stopping) = self.stopping()
            && stopping.join().is_err()
        {
            tracing::warn!("the server scope's runtime did not shut down cleanly");
        }
    }

    /// 在另一个线程上停下，返回那个线程。调用方可能在异步上下文里，那里不能阻塞地关运行时。
    pub(crate) fn stopping(&self) -> Option<JoinHandle<()>> {
        let runtime = self.lock().take()?;
        Some(std::thread::spawn(move || {
            runtime.shutdown_timeout(STOP_TIMEOUT);
        }))
    }

    pub(crate) fn stop_background(&self) {
        if let Some(runtime) = self.lock().take() {
            runtime.shutdown_background();
        }
    }

    fn lock(&self) -> MutexGuard<'_, Option<Runtime>> {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// 在异步上下文里直接丢掉 tokio 运行时会 panic。
impl Drop for ScopeRuntime {
    fn drop(&mut self) {
        self.stop_background();
    }
}

fn switched() -> CoreError {
    CoreError::NotReady {
        reason: l10n!(CATALOG, language(), "scope-switched-server"),
    }
}
