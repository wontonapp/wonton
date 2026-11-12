//! 把确认卡片交给主人的两种方式：核心接了 Matrix 时发进和主人的通知房间；没接时如实报“没法请主人确认”。

use futures_util::FutureExt;
use futures_util::future::BoxFuture;
use wonton_protocol::ConfirmationCard;

use super::cards;
use super::confirm::Notifier;
use crate::matrix::{Edited, NoticeRoom, Posted};

/// 卡片在哪：房间和事件。编辑只能发在原房间里。
fn encode(posted: &Posted) -> String {
    serde_json::json!([posted.room, posted.event]).to_string()
}

fn decode(handle: &str) -> Result<Posted, String> {
    let (room, event) = serde_json::from_str(handle)
        .map_err(|e| format!("确认卡片的句柄读不懂（{handle}）：{e}"))?;
    Ok(Posted { room, event })
}

impl Notifier for NoticeRoom {
    fn post<'a>(&'a self, card: &'a ConfirmationCard) -> BoxFuture<'a, Result<String, String>> {
        async move {
            self.send(&cards::content(card))
                .await
                .map(|posted| encode(&posted))
                .map_err(|e| format!("确认卡片发不到 Wonton 房间：{e}"))
        }
        .boxed()
    }

    fn update<'a>(
        &'a self,
        handle: &'a str,
        card: &'a ConfirmationCard,
    ) -> BoxFuture<'a, Result<Option<String>, String>> {
        async move {
            let edited = self
                .edit(&decode(handle)?, &cards::content(card))
                .await
                .map_err(|e| format!("更新确认卡片失败：{e}"))?;
            Ok(match edited {
                Edited::InPlace => None,
                Edited::Reposted(posted) => Some(encode(&posted)),
            })
        }
        .boxed()
    }
}

/// 核心没有接入 Matrix：卡片发不出去，敏感的调用一律不执行。
pub struct NoMatrix;

impl Notifier for NoMatrix {
    fn post<'a>(&'a self, _card: &'a ConfirmationCard) -> BoxFuture<'a, Result<String, String>> {
        async {
            Err("核心没有接入 Matrix（WONTON_MATRIX_URL），没法给主人发确认卡片".to_owned())
        }
        .boxed()
    }

    fn update<'a>(
        &'a self,
        _handle: &'a str,
        _card: &'a ConfirmationCard,
    ) -> BoxFuture<'a, Result<Option<String>, String>> {
        async { Ok(None) }.boxed()
    }
}
