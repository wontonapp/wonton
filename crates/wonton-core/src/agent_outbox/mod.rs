//! 出箱（ADR-0061 第 9、10 节）：核心把批准的消息和要取消的轮次排进这个 Bot 的队列，桥长轮询
//! `GET /agent/outbox` 取走，做完 `POST` 回报。Message 项落盘（[`store`]），核心重启后照常交给
//! 桥；CancelTurn 不落盘，重启时链都转 `Ended`，要取消的轮次随之失效。
//!
//! 取走即离开队列（`taken` 记着等回报）：另一次轮询不会再拿到队列里的同一项。但取走还没回报的
//! Message 项优先重发——桥拿同一项重发时用出箱项 ID 当 Matrix 事务 ID，服务器按事务 ID 去重，
//! 房间里只有一条（桥取走后崩溃重启、回报没送到，都靠这一条补上）。回报说没做成也照实结清：
//! 项从队列和存档里移除，审计记下失败类别，不再重发。

mod store;
#[cfg(test)]
mod tests;

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::sync::watch;
use wonton_protocol::{
    AGENT_OUTBOX_WAIT_SECS, BotRef, OutboxItem, OutboxKind, OutboxReceipt, OutboxResult,
};

/// 一条 Message 项结清（回报到了）时给审计的资料。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageSettled {
    pub bot: BotRef,
    pub recipient: String,
    pub body: String,
    pub result: OutboxResult,
}

type MessageSettledHandler = Arc<dyn Fn(&MessageSettled) + Send + Sync>;

#[derive(Default)]
struct State {
    queued: BTreeMap<BotRef, VecDeque<OutboxItem>>,
    /// 已取走、等回报的项。
    taken: HashMap<String, OutboxItem>,
    /// 取走还没回报的 Message 项，按取走次序：轮询先重发它们，别的项排队等。
    pending: VecDeque<(BotRef, String)>,
    /// 落盘的镜像：写盘成功才提交到内存，读回时从这里重建队列。
    persisted: Vec<store::Entry>,
    next: u64,
}

/// 出箱：每个 Bot 一条队列，长轮询挂在 `revision` 上。
pub struct Outbox {
    state: Mutex<State>,
    revision: watch::Sender<u64>,
    /// Message 项的存档路径；`None` 是没接存档（测试），只在内存里。
    store: Option<PathBuf>,
    on_settled: Mutex<Option<MessageSettledHandler>>,
}

impl Default for Outbox {
    fn default() -> Self {
        Self {
            state: Mutex::new(State::default()),
            revision: watch::channel(0).0,
            store: None,
            on_settled: Mutex::new(None),
        }
    }
}

impl Outbox {
    /// 打开出箱并接上存档：读回没投递完的 Message 项（坏文件报错，不顶替），
    /// 序号接着存档里最大的走，重启后不开重号。
    pub fn with_store(path: impl Into<PathBuf>) -> Result<Self, crate::plugin::store::StoreError> {
        let path = path.into();
        let (entries, next) = store::read(&path)?;
        // 项 ID 的序号必须不超过计数器：写进去的一定一致，坏档在这里现形。
        if store::max_id(&path, &entries)? > next {
            return Err(crate::plugin::store::StoreError::Corrupt {
                path: path.clone(),
                message: "项 ID 的序号超过计数器".into(),
            });
        }
        let mut queued: BTreeMap<BotRef, VecDeque<OutboxItem>> = BTreeMap::new();
        for entry in &entries {
            queued
                .entry(BotRef {
                    plugin_id: entry.plugin_id.clone(),
                    bot: entry.bot.clone(),
                })
                .or_default()
                .push_back(OutboxItem {
                    id: entry.id.clone(),
                    kind: OutboxKind::Message {
                        recipient: entry.recipient.clone(),
                        body: entry.body.clone(),
                    },
                });
        }
        Ok(Self {
            state: Mutex::new(State {
                queued,
                persisted: entries,
                next,
                ..State::default()
            }),
            revision: watch::channel(0).0,
            store: Some(path),
            on_settled: Mutex::new(None),
        })
    }

    /// 回报结清的 Message 项回调（审计用，核心启动时设置一次）。回调在锁外逐条调用；
    /// 不要再调回出箱，会死锁。
    pub fn on_message_settled(&self, handler: MessageSettledHandler) {
        *self.on_settled.lock().expect("出箱锁中毒") = Some(handler);
    }

    /// 给这个 Bot 排一项，返回项的 ID。别的 Bot 取不走它。Message 项先落盘，写不进去时
    /// 入队没有发生——调用方如实报错，不假装排上了。
    pub fn push(
        &self,
        bot: &BotRef,
        kind: OutboxKind,
    ) -> Result<String, crate::plugin::store::StoreError> {
        let mut state = self.state.lock().expect("出箱锁中毒");
        state.next += 1;
        let entry = match (&kind, self.store.as_ref()) {
            (OutboxKind::Message { recipient, body }, Some(path)) => {
                let entry = store::Entry {
                    id: format!("outbox-{}", state.next),
                    plugin_id: bot.plugin_id.clone(),
                    bot: bot.bot.clone(),
                    recipient: recipient.clone(),
                    body: body.clone(),
                };
                let mut entries = state.persisted.clone();
                entries.push(entry.clone());
                store::write(path, &entries, state.next)?;
                Some(entry)
            }
            _ => None,
        };
        let id = format!("outbox-{}", state.next);
        if let Some(entry) = entry {
            state.persisted.push(entry);
        }
        state
            .queued
            .entry(bot.clone())
            .or_default()
            .push_back(OutboxItem {
                id: id.clone(),
                kind,
            });
        self.bump();
        Ok(id)
    }

    /// 取这个 Bot 的下一项：先重发取走还没回报的 Message 项（按取走次序），再取队首。
    /// 取走的项进 `taken` 等回报；重发的那项留在 `taken` 里，回报到才结清。
    fn take(&self, bot: &BotRef) -> Option<OutboxItem> {
        let mut state = self.state.lock().expect("出箱锁中毒");
        if let Some((_, id)) = state.pending.iter().find(|(owner, _)| owner == bot) {
            return state.taken.get(id).cloned();
        }
        let queue = state.queued.get_mut(bot)?;
        let item = queue.pop_front()?;
        if queue.is_empty() {
            state.queued.remove(bot);
        }
        state.taken.insert(item.id.clone(), item.clone());
        if matches!(item.kind, OutboxKind::Message { .. }) {
            state.pending.push_back((bot.clone(), item.id.clone()));
        }
        Some(item)
    }

    /// 回报一项：只认取走过的项，重复回报按没做过处理（返回假）。
    pub(crate) fn receipt(&self, receipt: &OutboxReceipt) -> bool {
        let settled = {
            let mut state = self.state.lock().expect("出箱锁中毒");
            let Some(item) = state.taken.remove(&receipt.id) else {
                return false;
            };
            let bot = state
                .pending
                .iter()
                .find(|(_, id)| id == &receipt.id)
                .map(|(bot, _)| bot.clone());
            state.pending.retain(|(_, id)| id != &receipt.id);
            let OutboxKind::Message { recipient, body } = item.kind else {
                return true;
            };
            if let Some(path) = self.store.as_ref() {
                let entries: Vec<store::Entry> = state
                    .persisted
                    .iter()
                    .filter(|entry| entry.id != receipt.id)
                    .cloned()
                    .collect();
                // 写不进去只记错误：内存里已经结清，存档里留下的一项重启后会再交给桥，
                // 重发靠事务 ID 去重，房间里不会多出一条。
                if let Err(error) = store::write(path, &entries, state.next) {
                    tracing::error!(%error, id = %receipt.id, "出箱存档没写进去");
                } else {
                    state.persisted = entries;
                }
            }
            bot.map(|bot| MessageSettled {
                bot,
                recipient,
                body,
                result: receipt.result.clone(),
            })
        };
        let handler = self.on_settled.lock().expect("出箱锁中毒").clone();
        if let (Some(handler), Some(settled)) = (handler, settled) {
            handler(&settled);
        }
        true
    }

    /// 长轮询：队里有项立即给；没有就挂到有变化或者 [`AGENT_OUTBOX_WAIT_SECS`] 秒。
    /// 先订阅再看队列，中间进来的项不会漏掉。
    pub(crate) async fn next(&self, bot: &BotRef) -> Option<OutboxItem> {
        let mut watcher = self.revision.subscribe();
        let deadline = tokio::time::Instant::now() + Duration::from_secs(AGENT_OUTBOX_WAIT_SECS);
        loop {
            if let Some(item) = self.take(bot) {
                return Some(item);
            }
            match tokio::time::timeout_at(deadline, watcher.changed()).await {
                Ok(Ok(())) => {
                    watcher.borrow_and_update();
                }
                _ => return None,
            }
        }
    }

    fn bump(&self) {
        self.revision.send_modify(|revision| *revision += 1);
    }
}
