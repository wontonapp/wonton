//! 把确认卡片交给主人的两种方式：核心接了 Matrix 时发进和主人的通知房间；没接时如实报“没法请主人确认”。

use futures_util::FutureExt;
use futures_util::future::BoxFuture;
use wonton_protocol::ConfirmationCard;

use super::cards;
use super::confirm::Notifier;
use crate::matrix::NoticeRoom;

impl Notifier for NoticeRoom {
    fn post<'a>(&'a self, card: &'a ConfirmationCard) -> BoxFuture<'a, Result<String, String>> {
        async move {
            self.send(&cards::content(card))
                .await
                .map_err(|e| format!("确认卡片发不到 Wonton 房间：{e}"))
        }
        .boxed()
    }

    fn update<'a>(
        &'a self,
        handle: &'a str,
        card: &'a ConfirmationCard,
    ) -> BoxFuture<'a, Result<(), String>> {
        async move {
            self.edit(handle, &cards::content(card))
                .await
                .map_err(|e| format!("更新确认卡片失败：{e}"))
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
    ) -> BoxFuture<'a, Result<(), String>> {
        async { Ok(()) }.boxed()
    }
}
