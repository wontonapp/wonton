//! 发不出去也要送到的事件：删除的通知、收到请求后回的结果。事件和行的改动在同一次写盘里落下
//! （[`Contacts::commit`]），然后发；发不出去的留在 `core/contacts.json`，后台定时再发
//! （[`Contacts::flush_outbox`]），重启后照样发。
//!
//! 归属人自己点的请求、同意、拒绝不排队：发不出去当场失败，什么都不记（ADR-0050），他看得到。但发之前先把
//! 同一台服务器排着的事件发完：对方要按先后收到，不然迟到的删除会删掉之后新请求的那一行。
//!
//! 至少送到一次：对方收到重复的结果或删除时什么都不变（结果只认还在等的请求 ID，删除只删还在的行）。

use wonton_protocol::{ContactError, ContactEvent};

use super::store::{Queued, Stored};
use super::{ContactFailure, Contacts, PeerReach};

impl Contacts {
    pub(super) fn enqueue(stored: &mut Stored, server: &str, event: ContactEvent) {
        stored.outbox.push(Queued {
            server: server.to_owned(),
            event,
        });
    }

    /// 先发完给 `server` 排着的，再发这一条。调用方拿着 `ops`。
    pub(super) async fn send_in_order(
        &self,
        server: &str,
        event: ContactEvent,
    ) -> Result<(), ContactFailure> {
        self.flush_server(server).await?;
        self.ports.link.send(server, event).await
    }

    /// 按先后发给 `server` 排着的事件，送到一条去掉一条，遇到发不出去的停下。调用方拿着 `ops`。
    pub(super) async fn flush_server(&self, server: &str) -> Result<(), ContactFailure> {
        if self.ports.link.reach(server) == PeerReach::NotPaired {
            // 配对移除了也留着：重新配对后对方照样要知道这边删了谁。没配对时不试，什么都发不过去。
            let waiting = self.lock().outbox.iter().any(|q| q.server == server);
            if waiting {
                return Err(ContactFailure::new(
                    ContactError::NotPaired,
                    format!("服务器 {server} 没有和这台配对"),
                ));
            }
            return Ok(());
        }
        loop {
            let next = self
                .lock()
                .outbox
                .iter()
                .find(|q| q.server == server)
                .cloned();
            let Some(queued) = next else {
                return Ok(());
            };
            self.ports.link.send(server, queued.event.clone()).await?;
            // 写不进去时这条还在，下次再发一遍：对方收到重复的什么都不变。
            self.commit(|stored| {
                if let Some(at) = stored.outbox.iter().position(|q| *q == queued) {
                    stored.outbox.remove(at);
                }
            })?;
        }
    }

    /// 刚排进去的事件试着发一次；发不出去等后台再发。调用方拿着 `ops`。
    pub(super) async fn flush_or_later(&self, server: &str) {
        if let Err(e) = self.flush_server(server).await {
            tracing::info!(%server, error = %e, "联系人事件还没送到对方，稍后再发");
        }
    }

    /// 把排着的事件都发一遍。启动时和之后定时调。
    pub async fn flush_outbox(&self) {
        let _op = self.ops.lock().await;
        let mut servers: Vec<String> = self
            .lock()
            .outbox
            .iter()
            .map(|q| q.server.clone())
            .collect();
        servers.sort();
        servers.dedup();
        for server in servers {
            self.flush_or_later(&server).await;
        }
    }
}
