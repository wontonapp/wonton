//! 把确认卡片交出去的几种方式：核心接了 Matrix 时发进和主人的通知房间；没接时如实报“没法请主人确认”；
//! 发给家人的不进任何房间（[`ListOnly`]）。

use futures_util::FutureExt;
use futures_util::future::BoxFuture;
use serde_json::json;
use wonton_protocol::ConfirmationCard;

use super::cards;
use super::confirm::Notifier;
use std::sync::Arc;

use crate::matrix::{Edited, Login, Matrix, NoticeRoom, Posted};
use crate::plugin::languages::Languages;

/// 卡片在哪：房间和事件。编辑只能发在原房间里。
fn encode(posted: &Posted) -> String {
    serde_json::json!([posted.room, posted.event]).to_string()
}

fn decode(handle: &str) -> Result<Posted, String> {
    let (room, event) = serde_json::from_str(handle)
        .map_err(|e| format!("can't parse the confirmation card handle ({handle}): {e}"))?;
    Ok(Posted { room, event })
}

impl Notifier for NoticeRoom {
    fn post<'a>(&'a self, card: &'a ConfirmationCard) -> BoxFuture<'a, Result<String, String>> {
        async move {
            let langs = (self.languages)();
            self.send(&cards::content(card, &langs))
                .await
                .map(|posted| encode(&posted))
                .map_err(|e| format!("can't post the confirmation card to the Wonton room: {e}"))
        }
        .boxed()
    }

    fn update<'a>(
        &'a self,
        handle: &'a str,
        card: &'a ConfirmationCard,
    ) -> BoxFuture<'a, Result<Option<String>, String>> {
        async move {
            let langs = (self.languages)();
            let edited = self
                .edit(&decode(handle)?, &cards::content(card, &langs))
                .await
                .map_err(|e| format!("can't update the confirmation card: {e}"))?;
            Ok(match edited {
                Edited::InPlace => None,
                Edited::Reposted(posted) => Some(encode(&posted)),
            })
        }
        .boxed()
    }
}

/// 发在一个特定房间里的卡片（Bot 往主面板钉页的同意发在它自己的房间里）：用 Bot 自己刚登录的设备发，
/// 状态变化也在原卡片上改（核心自己不在这个房间里）。
pub(crate) struct BotRoom {
    pub matrix: Arc<Matrix>,
    pub login: Login,
    pub room: String,
    pub languages: Languages,
}

impl Notifier for BotRoom {
    fn post<'a>(&'a self, card: &'a ConfirmationCard) -> BoxFuture<'a, Result<String, String>> {
        async move {
            self.matrix
                .session(&self.login)
                .send_message(&self.room, &cards::content(card, &self.languages))
                .await
                .map(|event| {
                    encode(&Posted {
                        room: self.room.clone(),
                        event,
                    })
                })
                .map_err(|e| format!("can't post the confirmation card to the bot's room: {e}"))
        }
        .boxed()
    }

    fn update<'a>(
        &'a self,
        handle: &'a str,
        card: &'a ConfirmationCard,
    ) -> BoxFuture<'a, Result<Option<String>, String>> {
        async move {
            let posted = decode(handle)?;
            let content = cards::content(card, &self.languages);
            let mut edit = content.clone();
            if let Some(body) = content["body"].as_str().map(str::to_owned) {
                edit["body"] = json!(format!("* {body}"));
            }
            edit["m.new_content"] = content;
            edit["m.relates_to"] = json!({
                "rel_type": "m.replace",
                "event_id": posted.event,
            });
            self.matrix
                .session(&self.login)
                .send_message(&posted.room, &edit)
                .await
                .map(|_| None)
                .map_err(|e| format!("can't update the confirmation card: {e}"))
        }
        .boxed()
    }
}

/// 核心没有接入 Matrix：卡片发不出去，敏感的调用一律不执行。
pub struct NoMatrix;

impl Notifier for NoMatrix {
    fn post<'a>(&'a self, _card: &'a ConfirmationCard) -> BoxFuture<'a, Result<String, String>> {
        async {
            Err("the home server has no Matrix connection (WONTON_MATRIX_URL), so the confirmation card can't reach the owner".to_owned())
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

/// 不发进任何 Matrix 房间的卡片（发给家人的，ADR-0016）：核心记着，他的 App 从“等你确认”列表取，推送另走
/// [`super::RecipientPush`]。
pub struct ListOnly;

impl Notifier for ListOnly {
    fn post<'a>(&'a self, card: &'a ConfirmationCard) -> BoxFuture<'a, Result<String, String>> {
        async move { Ok(card.id.clone()) }.boxed()
    }

    fn update<'a>(
        &'a self,
        _handle: &'a str,
        _card: &'a ConfirmationCard,
    ) -> BoxFuture<'a, Result<Option<String>, String>> {
        async { Ok(None) }.boxed()
    }
}
