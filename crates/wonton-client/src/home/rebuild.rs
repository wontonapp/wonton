//! 整个重建 endpoint 的兜底：有证据表明本机 endpoint 死了时换新的（ADR-0032 §4）。

use std::sync::Arc;
use std::time::{Duration, Instant};

use wonton_protocol::DeviceNetwork;

use super::{HomeLink, Ticket};

/// 重建 endpoint 的冷却期。网络真断时检查会一直失败，重建救不了它，不能跟着失败的检查连环重建；冷却期内靠
/// 隧道客户端自己的退避重连。
const ENDPOINT_REBUILD_COOLDOWN: Duration = Duration::from_secs(30);

impl HomeLink {
    /// 本机 endpoint 救不回来时，整个重建（同一条连接信息，不重新配对）。返回有没有真的触发：被网络门或
    /// 冷却挡下时返回 false，调用方留着继续检查，冷却到期、网络回来后再试（ADR-0032 §4）。
    ///
    /// iroh 的 UDP socket 在网络切换的 rebind 失败后停在 Closed：重绑定要等下一次 major 变化，而 Android 的
    /// netmon 不产生系统事件，客户端的退避重连拨的全是死 socket。Android 模拟器实测（飞行模式往返）：
    /// rebind 撞上 `AddrInUse` 后 2 分钟内 4 次 major 都没救回来，重建 endpoint 后立刻恢复。
    ///
    /// 触发条件在调用方核对（ADR-0032 §4）：检查在失败，而配对信息里的中继一个都不在、设备网络又在位——
    /// 中继连接走同一个 UDP socket，跟着 socket 一起死；服务器死掉不影响中继，所以这组证据分得清本机死了还是
    /// 家里不通。iroh 把本机 socket 上的发送错误整个吞掉（拨号只会超时），没有更直接的证据可查。
    ///
    /// 这里只重建，别的什么都不动：状态、“连接中”和原因由新连接自己报出来，诊断窗口照旧计时，本地代理的
    /// 端口不变、凭据照常轮换，壳面按连接状态自己跟上。
    /// 触发点只做门控，重建放在独立任务里：establish 会关掉当前 link，把定时检查的调用者一起带走。
    pub(super) fn spawn_endpoint_rebuild(self: &Arc<Self>, trigger: &'static str) -> bool {
        {
            let mut last = self
                .endpoint_rebuilt_at
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            // 网络没了就不占冷却期：NetworkChanged 说网络回来时再重建。
            if self.lock_tracker().network() == Some(DeviceNetwork::None) {
                return false;
            }
            if last.is_some_and(|at| at.elapsed() < ENDPOINT_REBUILD_COOLDOWN) {
                return false;
            }
            *last = Some(Instant::now());
        }
        let home = self.clone();
        self.runtime.spawn(async move {
            home.rebuild_endpoint(trigger).await;
        });
        true
    }

    async fn rebuild_endpoint(self: Arc<Self>, trigger: &'static str) {
        // 触发到执行之间可能换过连接信息（重新配对、连别的服务器）：重建总是用现在这条。
        let Some(text) = self.lock_ticket().clone() else {
            return;
        };
        let Ok(ticket) = Ticket::parse(&text) else {
            return;
        };
        tracing::info!(
            trigger,
            "tunnel stays unreachable with a usable network; rebuilding the endpoint"
        );
        self.establish(ticket).await;
    }

    /// 只给测试：把当前 endpoint 关掉，模拟本机 socket 死掉（这条 socket 上的连接和中继一起失效）。
    #[cfg(feature = "test-hooks")]
    pub(crate) fn test_close_endpoint(self: &Arc<Self>) {
        let home = self.clone();
        self.runtime.spawn(async move {
            let Some((endpoint, _client)) = home.current_link().await else {
                return;
            };
            endpoint.close().await;
        });
    }

    /// 现在的连接信息里有没有中继表：重建的证据只在有中继时凑得齐（ADR-0032 §4）。
    pub(super) fn ticket_has_relays(&self) -> bool {
        self.lock_ticket()
            .as_deref()
            .and_then(|text| Ticket::parse(text).ok())
            .is_some_and(|ticket| ticket.has_relays())
    }
}
