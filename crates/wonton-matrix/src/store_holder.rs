//! 存储锁的持有者名（SDK 的 `CrossProcessLockConfig::MultiProcess`）。
//!
//! SDK 拿锁时只认持有者名：同一个存储上两个 Client 名字不同，新 Client 要等旧的租约过期（最多 500 ms），拿到锁时
//! 会换掉 Olm 机器：内存里的事件缓存和正等着房间密钥的解密都挂在旧机器上，设备身份也跟着换一套，对方看到的是同一台
//! 设备换了密钥，每条消息都要重新认一遍、重发一次密钥（#804、换服务器和隧道刚通时的实测）。
//!
//! 所以按存储取名：本进程还没在这个存储上建过 Client 时用 SDK 默认的 [`DEFAULT`]（上一个进程已经不在了，被杀后马上
//! 重开也能直接接手它留下的租约），之后一直沿用这个名字。同一个作用域里换会话都是先停掉旧会话再恢复
//! （`ChatLink::connect` 先 `forget_current`），旧 Client 不再写存储，沿用名字是安全的；它留下的引用（订阅里攥着的
//! 会话）也只是读。
//!
//! 锁记在各自的存储里，不同存储用同一个名字互不影响。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};

/// matrix-sdk 不指定时用的持有者名（`ClientBuilder` 里的私有常量）。
const DEFAULT: &str = "main";

/// 本进程给每个存储取过的名字。
static LAST: LazyLock<Mutex<HashMap<PathBuf, &'static str>>> = LazyLock::new(Mutex::default);

/// 开在 `store` 上的新 Client 拿哪个持有者名。只放内存的存储（`None`）不和别人共用，名字无所谓。
pub(crate) fn holder_for(store: Option<&Path>) -> &'static str {
    let Some(store) = store else { return DEFAULT };
    LAST.lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .entry(store.to_owned())
        .or_insert(DEFAULT)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 同一个存储上先后建的 Client 用同一个名字：SDK 不换 Olm 机器，设备身份不变。
    #[test]
    fn clients_on_one_store_keep_the_name() {
        let store = Path::new("/stores/one");
        let first = holder_for(Some(store));
        assert_eq!(holder_for(Some(store)), first);
    }

    #[test]
    fn the_first_client_on_a_store_uses_the_sdk_default() {
        assert_eq!(holder_for(Some(Path::new("/stores/first"))), DEFAULT);
    }

    /// 只放内存的存储没有名字冲突。
    #[test]
    fn an_in_memory_store_needs_no_holder() {
        assert_eq!(holder_for(None), DEFAULT);
    }
}
