//! 加星的先改后补：连点时请求按点的顺序一个个发，曲库最后的状态就是最后一次成功的那次（都失败就是点之前）。
//! 失败时不立刻改回去——还有在途的请求时改回去会和它打架——等这一项的请求都落地，有过失败才把界面改成曲库实际的状态。

use std::collections::HashMap;
use std::sync::Mutex;

use wonton_protocol::music::MusicTarget;

use super::lock;

#[derive(Default)]
pub(super) struct Stars {
    items: Mutex<HashMap<(String, MusicTarget), Item>>,
    /// 同一时刻只发一个加星请求，按点的顺序。
    order: tokio::sync::Mutex<()>,
}

struct Item {
    in_flight: u32,
    /// 曲库里现在的状态：点之前的，或者最后一次成功的。
    actual: bool,
    failed: bool,
}

pub(super) struct Ticket {
    key: (String, MusicTarget),
    starred: bool,
}

impl Stars {
    /// 点了一下。第一次点时，曲库里的状态就是点之前的（加星按钮是开关，点之前和要的相反）。
    pub(super) fn begin(&self, plugin_id: &str, target: &MusicTarget, starred: bool) -> Ticket {
        let key = (plugin_id.to_owned(), target.clone());
        let mut items = lock(&self.items);
        let item = items.entry(key.clone()).or_insert(Item {
            in_flight: 0,
            actual: !starred,
            failed: false,
        });
        item.in_flight += 1;
        Ticket { key, starred }
    }

    pub(super) async fn turn(&self) -> tokio::sync::MutexGuard<'_, ()> {
        self.order.lock().await
    }

    /// 一次请求落地。这一项没有在途请求、且中间失败过时，回曲库实际的状态，界面要改成它。
    pub(super) fn finish(&self, ticket: Ticket, succeeded: bool) -> Option<bool> {
        let mut items = lock(&self.items);
        let item = items.get_mut(&ticket.key)?;
        item.in_flight -= 1;
        if succeeded {
            item.actual = ticket.starred;
        } else {
            item.failed = true;
        }
        if item.in_flight > 0 {
            return None;
        }
        let item = items.remove(&ticket.key)?;
        item.failed.then_some(item.actual)
    }
}
