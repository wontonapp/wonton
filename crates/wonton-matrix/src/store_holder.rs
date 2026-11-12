//! 存储锁的持有者名（SDK 的 `CrossProcessLockConfig::MultiProcess`）。
//!
//! SDK 拿锁时只认持有者名：同一个存储上两个 Client 名字相同，另一个写回存储的明文、导入的房间密钥都被当成自己写的，
//! 内存里的事件缓存和 Olm 机器不重读，解不开的消息就一直解不开（#804）。名字不同，新 Client 又要等旧的租约过期
//! （最多 500 ms）：起步时拿不到锁，SDK 第一次拿到锁时会换掉 Olm 机器，正等着房间密钥的推送解密和重解密还挂在
//! 旧的那台上，等不到密钥。
//!
//! 所以按存储取名：
//! - 本进程还没在这个存储上建过 Client：用 SDK 默认的 [`DEFAULT`]。上一个进程（包括用默认名的旧版本）已经不在了，
//!   被杀后马上重开也能直接接手它留下的租约。
//! - 前一个 Client 所在的运行时已经关掉、它的任务一个都不会再跑：沿用它的名字。换服务器和临时作用域都先关掉旧作用域
//!   的运行时再开新的，走这一条。
//! - 前一个还可能在跑（同一个运行时里重建会话，旧 Client 被 SDK 留住）：换一个本进程独有的新名字。
//!
//! 锁记在各自的存储里，不同存储用同一个名字互不影响。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, LazyLock, Mutex, Weak};

use tokio::task::AbortHandle;

/// matrix-sdk 不指定时用的持有者名（`ClientBuilder` 里的私有常量）。
const DEFAULT: &str = "main";

/// 本进程发过几个新名字；和进程号一起保证不和别的进程、本进程别的 Client 撞名。
static NAMED: AtomicUsize = AtomicUsize::new(0);

/// 每个存储上最近一个 Client 的名字，和它所在运行时还在不在。
static LAST: LazyLock<Mutex<HashMap<PathBuf, Last>>> = LazyLock::new(Mutex::default);

struct Last {
    holder: String,
    /// 只有哨兵任务持有对应的 `Arc`：运行时关掉时任务被丢掉，这里就升级不了。
    running: Weak<()>,
    sentinel: AbortHandle,
}

/// 给在当前 tokio 运行时上新建、开在 `store` 上的 Client 取持有者名。只放内存的存储（`None`）不和别人共用，名字无所谓。
pub(crate) fn holder_for(store: Option<&Path>) -> String {
    let Some(store) = store else {
        return DEFAULT.to_owned();
    };
    let alive = Arc::new(());
    let running = Arc::downgrade(&alive);
    let sentinel = tokio::spawn(async move {
        let _alive = alive;
        std::future::pending::<()>().await;
    })
    .abort_handle();
    let mut last = LAST.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let holder = match last.remove(store) {
        None => DEFAULT.to_owned(),
        Some(previous) if previous.running.strong_count() == 0 => previous.holder,
        Some(previous) => {
            // 新 Client 用新名字，以后只和它比；旧的哨兵留着没用。
            previous.sentinel.abort();
            fresh()
        }
    };
    last.insert(
        store.to_owned(),
        Last {
            holder: holder.clone(),
            running,
            sentinel,
        },
    );
    holder
}

fn fresh() -> String {
    format!(
        "wonton-{}-{}",
        std::process::id(),
        NAMED.fetch_add(1, Ordering::Relaxed)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .build()
            .unwrap()
    }

    fn holder_on(runtime: &tokio::runtime::Runtime, store: &Path) -> String {
        runtime.block_on(async { holder_for(Some(store)) })
    }

    #[test]
    fn the_first_client_on_a_store_uses_the_sdk_default() {
        assert_eq!(holder_on(&runtime(), Path::new("/stores/first")), DEFAULT);
    }

    /// 沿用的是最近一个 Client 的名字：这里是同一运行时里重建过一次、已经换过名的那个。
    #[test]
    fn a_client_after_the_previous_runtime_stopped_keeps_its_name() {
        let store = Path::new("/stores/after-stop");
        let stopped = runtime();
        holder_on(&stopped, store);
        let latest = holder_on(&stopped, store);
        assert_ne!(latest, DEFAULT);
        stopped.shutdown_timeout(std::time::Duration::from_secs(5));
        assert_eq!(holder_on(&runtime(), store), latest);
    }

    #[test]
    fn a_client_beside_a_running_one_gets_its_own_name() {
        let store = Path::new("/stores/beside");
        let shared = runtime();
        let old = holder_on(&shared, store);
        let new = holder_on(&shared, store);
        assert_ne!(new, old);
        let elsewhere = holder_on(&runtime(), store);
        assert_ne!(elsewhere, new, "旧运行时还开着");
    }

    /// 锁记在各自的存储里：一个存储上的 Client 还在跑，不影响另一个存储第一次用默认名。
    #[test]
    fn stores_are_named_independently() {
        let shared = runtime();
        holder_on(&shared, Path::new("/stores/a"));
        assert_eq!(holder_on(&shared, Path::new("/stores/b")), DEFAULT);
    }
}
