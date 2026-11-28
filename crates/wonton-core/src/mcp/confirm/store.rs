//! 确认表本身：等回答的卡片、放行时段和列表版本，以及读写它用的锁。

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tokio::sync::{oneshot, watch};
use wonton_protocol::{ConfirmationCard, SecretInput};

pub use super::types::DEFAULT_WINDOW;
use super::types::{Chosen, Notifier, Recipients};
use super::windows::Persisted;
use crate::plugin::PluginId;

/// 回答过、过期的确认留这么久，之后再回答得到“不存在”。
const KEEP_FINISHED: Duration = Duration::from_secs(24 * 3600);

pub(in crate::mcp) struct Slot {
    pub(super) caller: PluginId,
    /// 这张卡片的冻结收件人。
    pub(super) recipients: Recipients,
    /// 这张卡片能打开放行时段时，时段管哪个插件。
    pub(super) scope: Option<String>,
    /// 见 [`Ticket::window`](super::Ticket::window)。
    pub(super) released_by: Option<String>,
    pub(super) card: ConfirmationCard,
    pub(super) waiter: Option<oneshot::Sender<Chosen>>,
    /// 这张卡片要填的密钥（ADR-0064）；不填密钥的卡片是空的。
    pub(super) secret_inputs: Vec<SecretInput>,
    /// 不再等回答的时刻（Unix 毫秒）：答了、拒了、过期或被取代。结束的卡片按 [`KEEP_FINISHED`] 留这么久。
    pub(super) ended: Option<u64>,
}

impl Slot {
    pub(super) fn includes(&self, recipient: Option<&str>) -> bool {
        self.recipients.includes(recipient)
    }
}

#[derive(Default)]
pub(in crate::mcp) struct Table {
    pub(super) slots: HashMap<String, Slot>,
    /// 生效中的放行时段：（Bot，插件）→ 打开它的卡片。其余状态在卡片的 `window` 里。
    pub(super) windows: HashMap<(PluginId, String), String>,
    /// 按收件人放宽的时段（ADR-0024，`wonton__send_message`）：（Bot，收件人）→ 打开它的卡片。
    pub(super) recipient_windows: HashMap<(PluginId, String), String>,
    /// 上次运行留下、卡片还没改成“已结束”的时段。
    pub(super) leftovers: HashMap<String, Persisted>,
}

pub struct Confirmations {
    pub(super) notifier: Arc<dyn Notifier>,
    pub(super) ttl: Duration,
    pub(super) window: Duration,
    /// 结束的卡片留多久（`KEEP_FINISHED`，测试里可改短）。
    pub(super) keep: Duration,
    /// 记着哪些卡片显示着时段，核心重启后才改得了它们。判定不读它。
    pub(super) windows_file: Option<PathBuf>,
    pub(super) table: Mutex<Table>,
    /// 列表的版本：任何一张卡片变了就加一，网关的长轮询挂在这个通道上（[`wonton_protocol::CONFIRMATIONS_PATH`]）。
    pub(super) revision: watch::Sender<u64>,
    /// 确认状态变化要唤醒的活动表。挂它的 Bus 可能重建（测试里共用 Confirmations），
    /// 所以允许换接：新 Bus 的活动表才是当前事实源。
    pub(super) activity: RwLock<Option<Arc<crate::bot_activity::BotActivity>>>,
}

pub(in crate::mcp) fn now_ms(at: SystemTime) -> u64 {
    at.duration_since(UNIX_EPOCH)
        .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or_default()
}

impl Confirmations {
    /// `ttl`：卡片发出后多久没回答就过期。
    pub fn new(notifier: Arc<dyn Notifier>, ttl: Duration) -> Self {
        Self {
            notifier,
            ttl,
            window: DEFAULT_WINDOW,
            keep: KEEP_FINISHED,
            windows_file: None,
            table: Mutex::new(Table::default()),
            revision: watch::Sender::new(0),
            activity: RwLock::new(None),
        }
    }

    pub fn ttl(&self) -> Duration {
        self.ttl
    }

    /// 结束的卡片留多久（测试里改短，好验证“过了保留期取不到”）。
    #[cfg(test)]
    #[must_use]
    pub fn with_keep_finished(mut self, keep: Duration) -> Self {
        self.keep = keep;
        self
    }

    pub(super) fn lock(&self) -> std::sync::MutexGuard<'_, Table> {
        self.table.lock().expect("确认表锁中毒")
    }

    /// 卡片变了：版本加一，挂着的长轮询知道。状态变化只改确认表，不再改任何 Matrix 消息。
    pub(in crate::mcp) fn bump(&self) {
        self.revision.send_modify(|revision| *revision += 1);
    }

    /// 清掉结束超过保留期的卡片。
    pub(super) fn prune(&self, table: &mut Table, now: SystemTime) {
        let before = table.slots.len();
        let open: Vec<String> = table.windows.values().cloned().collect();
        table.slots.retain(|id, slot| {
            slot.waiter.is_some()
                || open.contains(id)
                || slot.ended.is_none_or(|ended| {
                    Duration::from_millis(now_ms(now).saturating_sub(ended)) < self.keep
                })
        });
        if table.slots.len() != before {
            self.bump();
        }
    }
}
