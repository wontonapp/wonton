//! 定时确认隧道真的通：不通就断开换新连接，连不上接着重试；有证据表明本机 endpoint 死了时触发重建
//! （ADR-0032 §2、§4）。

use std::sync::Weak;
use std::time::Duration;

use iroh::Endpoint;
use wonton_tunnel::{TunnelClient, TunnelError, TunnelStatus};

use super::HomeLink;

/// 多久确认一次隧道真的通。iroh 每 5 秒发一次心跳，无线电反正醒着，这个检查不多耗电。
const CHECK_INTERVAL: Duration = Duration::from_secs(5);
/// 等回应的时限。n0 中继的往返在负载下出现过 2.8 秒（ADR-0004），大文件传输时还要排队；判错的代价是断开正在传的
/// 流，所以取得宽，不通的时候最晚 `CHECK_INTERVAL + CHECK_DEADLINE` 发现。
const CHECK_DEADLINE: Duration = Duration::from_secs(8);
/// 连续失败的检查之间至少隔这么久。重试的节奏由隧道客户端的退避决定，这里只防止出错时空转。
const CHECK_FLOOR: Duration = Duration::from_millis(200);
/// 连续多少次"检查失败且一个中继都不在"就整个重建 endpoint（ADR-0032 §4）。
const RELAY_LOST_BEFORE_REBUILD: u32 = 2;
/// 把“网络可能变了”交给 iroh 的时限。正常时 actor 的信箱有空位，立刻交上。
const NUDGE_DEADLINE: Duration = Duration::from_secs(2);

/// 一直确认隧道真的通：不通就断开换新连接，连不上就接着重试，不等 App 回到前台，也不等下一个请求。连接状态和路径
/// 因此如实（#154）。在后台时停下。服务端不认这台设备、或者两端协议不匹配时重试也没用，等主人处理。只持有弱引用，核心
/// 释放后自动结束。
pub(super) async fn keep_checking(home: Weak<HomeLink>, endpoint: Endpoint, client: TunnelClient) {
    let Some((mut background, mut periodic_checks)) = home.upgrade().map(|home| {
        (
            home.background.subscribe(),
            home.periodic_checks.subscribe(),
        )
    }) else {
        return;
    };
    let mut status = client.watch_status();
    let mut wait = CHECK_INTERVAL;
    let mut relay_lost = 0u32;
    loop {
        // 请求在隧道流上等不到服务端回执时会催一下：连接可能已经死了，
        // 立即确认，不等满一个周期。催多次只存一个许可，不会连环检查。
        tokio::select! {
            _ = tokio::time::sleep(wait) => {}
            _ = client.wait_for_check_nudge() => {}
        }
        if background.wait_for(|background| !background).await.is_err() {
            return;
        }
        if periodic_checks.wait_for(|on| *on).await.is_err() {
            return;
        }
        if status
            .wait_for(|status| !needs_the_owner(status))
            .await
            .is_err()
        {
            return;
        }
        if home.strong_count() == 0 {
            return;
        }
        let result = client.probe(CHECK_DEADLINE).await;
        let Some(link) = home.upgrade() else {
            return;
        };
        link.recount();
        wait = match result {
            Ok(_) => {
                relay_lost = 0;
                CHECK_INTERVAL
            }
            Err(err) => {
                tracing::info!(code = err.code(), "tunnel check failed");
                // 检查失败本身不是重建的证据：服务器关机、断电时它也失败，网络却好好的。
                // 本机 socket 死了的证据是配对信息里的中继一个都不在（中继连接走同一个 UDP socket，跟着
                // 一起死；而服务器死掉不影响中继，ADR-0032 §4）。endpoint 关闭后中继状态整个清空，
                // relay_check 只会说 Unknown，所以判"没有中继在连"，不判某个具体取值。连续两轮失败且
                // 网络又在位：重建 endpoint。旧 link 会被 establish 关掉，这个检查循环跟着结束。
                if link.ticket_has_relays()
                    && super::relay_check(&endpoint) != wonton_protocol::ConnectionCheck::Ok
                {
                    relay_lost += 1;
                } else {
                    relay_lost = 0;
                }
                if relay_lost >= RELAY_LOST_BEFORE_REBUILD
                    && link.spawn_endpoint_rebuild("checks failing while no relay is reachable")
                {
                    return;
                }
                // 没触发（冷却期内、或网络没了不占冷却期）：留着这个检查循环，streak 已经够数，下一次
                // 失败的检查再试一次 spawn，冷却到期、网络回来后自然通过。
                // 可能是网络变了，让 iroh 重新检查网卡和路径。
                nudge_network(&endpoint).await;
                CHECK_FLOOR
            }
        };
    }
}

pub(super) fn needs_the_owner(status: &TunnelStatus) -> bool {
    matches!(
        status,
        TunnelStatus::Disconnected(TunnelError::Unauthorized | TunnelError::Protocol { .. })
    )
}

/// 让 iroh 重新检查网卡和路径，限时交上，交不上就算了。
///
/// iroh 的 socket actor 处理这条通知时等网络监视收下；网络监视在 Android 上每次重新检查都 fork 执行
/// `ip route`，子进程卡死时它再也收不下（见 `bind.rs`），socket actor 跟着停在这里，信箱满了以后谁交这条通知
/// 谁就永远等着。不限时的话，检查循环、回前台和“重试”都会停在这一步，连接再也不重连。
pub(super) async fn nudge_network(endpoint: &Endpoint) {
    if tokio::time::timeout(NUDGE_DEADLINE, endpoint.network_change())
        .await
        .is_err()
    {
        tracing::warn!("iroh did not take the network change in time");
    }
}
