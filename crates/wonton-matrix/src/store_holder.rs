//! 存储锁的持有者名（SDK 的 `CrossProcessLockConfig::MultiProcess`）。
//!
//! SDK 拿锁时只认持有者名：同一个存储上两个 Client 名字相同，另一个写回存储的明文、导入的房间密钥都被当成自己写的，
//! 内存里的事件缓存和 Olm 机器不重读，解不开的消息就一直解不开（#804）。名字不同，新 Client 拿到锁时锁会被标脏，
//! SDK 换掉 Olm 机器、从存储重读，才看得见另一个 Client 刚写回的东西。
//!
//! 换名字不是不要钱：新 Client 要等旧的租约过期（最多 500 ms）才拿得到锁，拿到锁还要整台 Olm 机器重读一遍。
//! 换服务器、隧道刚通、先用本机聊天再恢复这些时候，旧 Client 已经不写了，再换名只是白付这份钱（#1047）。
//!
//! 所以按「这个存储上还有没有活着的 Client」取名，而不是按运行时：有，就换一个本进程独有的新名字，让 generation
//! 递增、彼此看见写入；没有，就沿用这个存储上一个 Client 的名字，不重载、不等租约。进程重启后没有记录，按进程独有的
//! 新名字开始，和同时在跑的别的进程区分开。
//!
//! 判活靠 [`StoreLease`]：持有者活着它就活着，Client 落了它就落。锁记在各自的存储里，不同存储用同一个名字互不影响。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, LazyLock, Mutex, Weak};
use std::time::{SystemTime, UNIX_EPOCH};

/// 只放内存的存储（`None`）不和别人共用，名字无所谓。
const DEFAULT: &str = "main";

/// 本进程发过几个新名字；和进程号、启动时刻一起保证不和别的进程、本进程别的 Client 撞名。
static NAMED: AtomicUsize = AtomicUsize::new(0);

/// 本进程启动时取一次的熵：容器里每个进程的 PID 都是 1，只靠 PID 的话升级时新旧两个容器在同一个卷上第一次都叫
/// `wonton-1-0`，SDK 当成同一个持有者、generation 不加、事件和 Olm 不重读（#1047 的 bug 回来）。取启动时刻的
/// 纳秒：同一个进程里只取一次，名字对同进程稳定、对别的进程不同。
static NONCE: LazyLock<u64> = LazyLock::new(|| {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_nanos() as u64)
});

/// 每个存储上最近一个 Client 的名字，和它还在不在。
static LAST: LazyLock<Mutex<HashMap<PathBuf, Last>>> = LazyLock::new(Mutex::default);

struct Last {
    holder: String,
    /// 只有 [`StoreLease`] 持有那个 `Arc` 的强引用：Client 落了这里就升级不了。
    alive: Weak<()>,
}

/// 一个 Client 在这个存储上的租约：活着期间，同存储的新 Client 会换新名字；落掉以后，新 Client 接着用它的名字。
///
/// 拿到就得留着（和 Client 同寿）：丢早了，同进程并发又会撞名。
#[must_use = "租约落掉后，同存储的下一个 Client 会以为没有别人、沿用这个名字"]
pub(crate) struct StoreLease {
    _alive: Arc<()>,
}

/// 给开在 `store` 上的新 Client 取持有者名，返回的租约要和这个 Client 同寿。
pub(crate) fn holder_for(store: Option<&Path>) -> (String, Option<StoreLease>) {
    let Some(store) = store else {
        return (DEFAULT.to_owned(), None);
    };
    let alive = Arc::new(());
    let mut last = LAST.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let holder = match last.get(store) {
        // 还有活着的 Client：换一个本进程独有的新名字，让 generation 递增。
        Some(previous) if previous.alive.strong_count() > 0 => fresh(),
        // 上一个 Client 已经不在了：沿用它的名字，不重载、不等租约。
        Some(previous) => previous.holder.clone(),
        // 本进程第一次见这个存储：用进程独有的名字，不和同时在跑的别的进程撞。
        None => fresh(),
    };
    last.insert(
        store.to_owned(),
        Last {
            holder: holder.clone(),
            alive: Arc::downgrade(&alive),
        },
    );
    (holder, Some(StoreLease { _alive: alive }))
}

fn fresh() -> String {
    format!(
        "wonton-{}-{:016x}-{}",
        std::process::id(),
        *NONCE,
        NAMED.fetch_add(1, Ordering::Relaxed)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 本进程第一次见这个存储：名字是进程独有的，不是 SDK 默认的 `main`（那个会和别的进程撞）。
    #[test]
    fn the_first_client_on_a_store_gets_a_process_unique_name() {
        let (holder, _lease) = holder_for(Some(Path::new("/stores/first")));
        assert_ne!(holder, DEFAULT);
    }

    /// 容器里每个进程的 PID 都是 1：名字不能只有 `<pid>-<计数器>`，否则升级时新旧两个容器在同一个卷上第一次都叫
    /// `wonton-1-0`，SDK 当成同一个持有者、generation 不加、事件和 Olm 不重读（#1047 的 bug 回来）。
    #[test]
    fn the_name_is_not_only_the_pid_and_a_counter() {
        let (holder, _lease) = holder_for(Some(Path::new("/stores/pid-collision")));
        let old_form = holder
            .strip_prefix(&format!("wonton-{}-", std::process::id()))
            .is_some_and(|rest| rest.chars().all(|c| c.is_ascii_digit()));
        assert!(!old_form, "名字只有 PID 和计数器，容器里会撞：{holder}");
    }

    /// 挨着活着的 Client 建的：换一个本进程独有的新名字，generation 才会递增。
    #[test]
    fn a_client_beside_a_running_one_gets_its_own_name() {
        let store = Path::new("/stores/beside");
        let (old, _old_lease) = holder_for(Some(store));
        let (new, _new_lease) = holder_for(Some(store));
        assert_ne!(new, old);
    }

    /// 前一个 Client 落了：沿用它的名字，不重载 Olm 机器、不等租约。
    #[test]
    fn a_client_after_the_previous_one_is_gone_keeps_the_name() {
        let store = Path::new("/stores/after-stop");
        let (held, lease) = holder_for(Some(store));
        drop(lease);
        let (next, _next_lease) = holder_for(Some(store));
        assert_eq!(next, held);
    }

    /// 锁记在各自的存储里：一个存储上的 Client 换名，不影响另一个存储第一次用默认名。
    #[test]
    fn stores_are_named_independently() {
        let (a, _a) = holder_for(Some(Path::new("/stores/a")));
        let (b, _b) = holder_for(Some(Path::new("/stores/b")));
        assert_ne!(a, b);
    }

    /// 只放内存的存储没有名字冲突。
    #[test]
    fn an_in_memory_store_needs_no_holder() {
        let (holder, lease) = holder_for(None);
        assert_eq!(holder, DEFAULT);
        assert!(lease.is_none());
    }
}
