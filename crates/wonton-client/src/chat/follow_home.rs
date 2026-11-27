//! 跟着家里的连接走（ADR-0042 的每台服务器一份作用域）：本机有那台服务器存下的会话就先恢复出来，不等隧道；本地代理
//! 就绪或换了就换成带代理的登录（重新）登录；隧道恢复连接时，登录失败过就再试一次。

use std::sync::Weak;

use tokio::sync::watch;
use wonton_protocol::{HomeStatus, HomeViewModel, ProxySettings};

use super::ChatLink;

/// 隧道抖动时先等它稳这么久：一秒能换好几次代理，不该为每个中间值都断/登一次会话。
const PROXY_SETTLE: std::time::Duration = std::time::Duration::from_millis(150);
/// 最多等几个间隔（一直不稳就按最后看到的值先动，下一次变化还会再来）。
const PROXY_SETTLE_ROUNDS: u8 = 3;

/// 服务器作用域释放后任务跟着停（`Weak` 升级失败就返回）。
pub(super) async fn run(chat: Weak<ChatLink>, mut home: watch::Receiver<HomeViewModel>) {
    // 上次看过的服务器和代理，两者一起算：换服务器时先换 server_id、代理也可能还是空的。
    let mut seen: Option<(Option<String>, Option<ProxySettings>)> = None;
    let mut connected = false;
    loop {
        let (key, now_connected) = {
            let view = home.borrow_and_update();
            (
                (view.server_id.clone(), view.proxy.clone()),
                view.status == HomeStatus::Connected,
            )
        };
        let Some(link) = chat.upgrade() else { return };
        let reconnected = now_connected && !connected;
        connected = now_connected;
        if seen.as_ref() != Some(&key) {
            // 等隧道稳一小会儿再用最后的代理：中间的地址不各换一次会话。等不及或一直不稳就按最后看到的值走。
            // 等待里只看不消费（`borrow`），等完重算一次 key 和连接状态：另一个变化还没被标记过，下一次循环
            // 会照它再来一遍。
            let (mut latest, mut latest_connected) = key.clone();
            for _ in 0..PROXY_SETTLE_ROUNDS {
                tokio::time::sleep(PROXY_SETTLE).await;
                let (now, now_connected) = {
                    let view = home.borrow();
                    (
                        (view.server_id.clone(), view.proxy.clone()),
                        view.status == HomeStatus::Connected,
                    )
                };
                let changed = now != latest;
                latest = now;
                latest_connected = now_connected;
                if !changed {
                    break;
                }
            }
            seen = Some(latest.clone());
            connected = latest_connected;
            match latest.1 {
                Some(next) => {
                    link.runtime
                        .spawn(link.clone().connect(super::connect::Target::Home(next)));
                }
                // 代理没了（换服务器、重建连接）：先把本机存下的会话恢复出来，隧道通到哪儿算哪儿。
                None => {
                    link.disconnect().await;
                    link.runtime
                        .spawn(link.clone().connect(super::connect::Target::HomeOffline));
                }
            };
        } else if reconnected {
            link.foregrounded();
        }
        drop(link);
        if home.changed().await.is_err() {
            return;
        }
    }
}
