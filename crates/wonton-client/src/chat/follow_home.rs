//! 跟着家里的连接走（ADR-0042 的每台服务器一份作用域）：本地代理就绪或换了就（重新）登录；隧道恢复连接时，
//! 登录失败过就再试一次。

use std::sync::Weak;

use tokio::sync::watch;
use wonton_protocol::{HomeStatus, HomeViewModel, ProxySettings};

use super::ChatLink;

/// 服务器作用域释放后任务跟着停（`Weak` 升级失败就返回）。
pub(super) async fn run(chat: Weak<ChatLink>, mut home: watch::Receiver<HomeViewModel>) {
    let mut proxy: Option<ProxySettings> = None;
    let mut connected = false;
    loop {
        let (next_proxy, now_connected) = {
            let view = home.borrow_and_update();
            (view.proxy.clone(), view.status == HomeStatus::Connected)
        };
        let Some(link) = chat.upgrade() else { return };
        let reconnected = now_connected && !connected;
        connected = now_connected;
        if next_proxy != proxy {
            proxy.clone_from(&next_proxy);
            match next_proxy {
                Some(next) => {
                    link.runtime
                        .spawn(link.clone().connect(super::connect::Target::Home(next)));
                }
                None => link.disconnect().await,
            }
        } else if reconnected {
            link.foregrounded();
        }
        drop(link);
        if home.changed().await.is_err() {
            return;
        }
    }
}
