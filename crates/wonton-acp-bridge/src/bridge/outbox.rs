//! 出箱 Message 项的投递（ADR-0061 第 9 节）：把正文以 Bot 的身份发进和收件人的私聊，
//! 用出箱项 ID 当 Matrix 事务 ID——同一项重发时服务器按事务 ID 去重，房间里只有一条。
//! 没有能用的私聊时报 `no_room`；带 Bot 联系人标记建私聊等 ADR-0083 的房间表，现在先这样报。

use std::sync::Arc;

use futures_util::future::BoxFuture;
use wonton_matrix::{MatrixSession, OwnedUserId};
use wonton_protocol::{OutboxFailure, OutboxResult};

use crate::error::{Detail, Error};

/// 私聊投递做不成的原因，出箱回报的类别直接用它。
#[derive(Debug, thiserror::Error)]
pub(crate) enum DmError {
    /// 没有和收件人的已加入私聊（没建过、只有邀请、还没同步进来都算）。
    #[error("没有和收件人的私聊")]
    NoRoom,
    /// 发送没做成：传输、加密或服务器拒了。
    #[error("{0}")]
    Send(Error),
}

/// 和收件人的私聊。生产实现走 [`MatrixSession`]，测试用内存替身断言事务 ID 和正文。
/// 返回装 futures：放进 [`Bridge`](crate::bridge::Bridge) 时用 `Arc<dyn Dm>`，测试换替身。
pub(crate) trait Dm: Send + Sync {
    /// 把 `body` 以 Bot 的身份发进和 `recipient`（本服务器用户名的 localpart）的私聊，
    /// `txn_id` 原样交给服务器去重，返回那条消息的事件 ID。
    fn send_to<'a>(
        &'a self,
        recipient: &'a str,
        txn_id: &'a str,
        body: &'a str,
    ) -> BoxFuture<'a, Result<String, DmError>>;
}

impl Dm for Arc<dyn Dm> {
    fn send_to<'a>(
        &'a self,
        recipient: &'a str,
        txn_id: &'a str,
        body: &'a str,
    ) -> BoxFuture<'a, Result<String, DmError>> {
        (**self).send_to(recipient, txn_id, body)
    }
}

/// 投递结果 → 出箱回报。发送失败只在这里记一条日志：类别如实回报，重发由核心的出箱管。
pub(crate) fn result_of(outcome: Result<String, DmError>) -> OutboxResult {
    match outcome {
        Ok(event_id) => OutboxResult::Sent { event_id },
        Err(DmError::NoRoom) => OutboxResult::Failed {
            category: OutboxFailure::NoRoom,
        },
        Err(DmError::Send(error)) => {
            tracing::warn!(%error, "发进私聊没做成");
            OutboxResult::Failed {
                category: OutboxFailure::Other,
            }
        }
    }
}

/// 收件人的 Matrix 用户 ID：家人用户名就是本服务器的 localpart。拼不出来按没有私聊报。
fn peer(server_name: &str, recipient: &str) -> Result<OwnedUserId, DmError> {
    format!("@{recipient}:{server_name}")
        .parse()
        .map_err(|_| DmError::NoRoom)
}

/// 生产实现：私聊按 `m.direct` 认，挑已加入的那一间发。
#[derive(Clone)]
pub(crate) struct MatrixDm {
    session: Arc<MatrixSession>,
}

impl MatrixDm {
    pub(crate) fn new(session: Arc<MatrixSession>) -> Self {
        Self { session }
    }

    fn server_name(&self) -> String {
        self.session.user_id().server_name().to_string()
    }
}

impl Dm for MatrixDm {
    fn send_to<'a>(
        &'a self,
        recipient: &'a str,
        txn_id: &'a str,
        body: &'a str,
    ) -> BoxFuture<'a, Result<String, DmError>> {
        Box::pin(self.send_to_dm(recipient, txn_id, body))
    }
}

impl MatrixDm {
    async fn send_to_dm(
        &self,
        recipient: &str,
        txn_id: &str,
        body: &str,
    ) -> Result<String, DmError> {
        let peer = peer(&self.server_name(), recipient)?;
        let rooms = self
            .session
            .direct_chat_rooms_with(&peer)
            .await
            .map_err(|error| DmError::Send(Error::Upstream(Detail::data(error.to_string()))))?;
        // 受邀的房间发不了，没同步进客户端的房间拿不到，都跳过。
        for room_id in rooms {
            let Some(room) = self.session.client().get_room(&room_id) else {
                continue;
            };
            if !matches!(room.state(), matrix_sdk::RoomState::Joined) {
                continue;
            }
            let sent = room
                .send_raw(
                    "m.room.message",
                    serde_json::json!({"msgtype": "m.text", "body": body}),
                )
                .with_transaction_id(txn_id.into())
                .await
                .map_err(|error| DmError::Send(Error::Upstream(Detail::data(error.to_string()))))?;
            return Ok(sent.response.event_id.to_string());
        }
        Err(DmError::NoRoom)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// 记下每次发送，按预设返回结果。别处的桥测试用它顶住出箱循环。
    pub(crate) struct FakeDm {
        pub(crate) sends: std::sync::Mutex<Vec<(String, String, String)>>,
        pub(crate) event_id: Option<String>,
    }

    impl FakeDm {
        /// 别的桥测试不带 Message 项：私聊一律报没有。
        pub(crate) fn for_bridge() -> Self {
            Self::no_room()
        }

        pub(crate) fn ok(event_id: &str) -> Self {
            Self {
                sends: std::sync::Mutex::new(Vec::new()),
                event_id: Some(event_id.to_owned()),
            }
        }

        pub(crate) fn no_room() -> Self {
            Self {
                sends: std::sync::Mutex::new(Vec::new()),
                event_id: None,
            }
        }
    }

    impl Dm for FakeDm {
        fn send_to<'a>(
            &'a self,
            recipient: &'a str,
            txn_id: &'a str,
            body: &'a str,
        ) -> BoxFuture<'a, Result<String, DmError>> {
            self.sends.lock().unwrap().push((
                recipient.to_owned(),
                txn_id.to_owned(),
                body.to_owned(),
            ));
            match &self.event_id {
                Some(event_id) => Box::pin(std::future::ready(Ok(event_id.clone()))),
                None => Box::pin(std::future::ready(Err(DmError::NoRoom))),
            }
        }
    }

    #[test]
    fn sent_receipt_carries_the_event_id() {
        let dm = FakeDm::ok("$abc:home.example");
        tokio::runtime::Runtime::new().unwrap().block_on(async {
            let outcome = result_of(dm.send_to("mom", "outbox-1", "周末回来吗").await);
            assert_eq!(
                outcome,
                OutboxResult::Sent {
                    event_id: "$abc:home.example".into()
                }
            );
        });
        assert_eq!(
            *dm.sends.lock().unwrap(),
            vec![("mom".into(), "outbox-1".into(), "周末回来吗".into())]
        );
    }

    #[test]
    fn a_missing_dm_reports_no_room() {
        let dm = FakeDm::no_room();
        tokio::runtime::Runtime::new().unwrap().block_on(async {
            let outcome = result_of(dm.send_to("stranger", "outbox-2", "在吗").await);
            assert_eq!(
                outcome,
                OutboxResult::Failed {
                    category: OutboxFailure::NoRoom
                }
            );
        });
    }

    #[test]
    fn the_peer_is_a_local_user_id() {
        assert_eq!(
            peer("home.example", "mom").unwrap().to_string(),
            "@mom:home.example"
        );
        // 用户名带冒号拼不出用户 ID：按没有私聊报。
        assert!(matches!(peer("home.example", "a:b"), Err(DmError::NoRoom)));
    }
}

/// 对真实 Tuwunel 的投递（ADR-0061 第 9 节）：桥按 `m.direct` 认私聊、带事务 ID 发送，
/// 收件人读到一条 `m.text`；同一事务重发服务器去重，房间里还是一条。
/// 跑法：`cargo test -p wonton-acp-bridge --lib outbox -- --include-ignored`（需要 Docker）。
#[cfg(test)]
mod tuwunel {
    use std::sync::Arc;
    use std::time::Duration;

    use futures_util::FutureExt;
    use wonton_matrix::{Message, MessageBody};

    use super::{Dm, MatrixDm};

    fn unique(name: &str) -> String {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        format!("{name}-{}", nanos % 1_000_000_000)
    }

    /// 每 200ms 看一眼，最多 10 秒。
    async fn eventually<T>(what: &str, mut check: impl FnMut() -> Option<T>) -> T {
        for _ in 0..50 {
            if let Some(value) = check() {
                return value;
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        panic!("等了 10 秒还没有：{what}");
    }

    #[tokio::test]
    #[ignore = "需要 Docker，用 --include-ignored 运行"]
    async fn an_outbox_message_reaches_the_dm_once() {
        let server = wonton_matrix::testkit::TestServer::start().await;
        let bot = server.login(&unique("bot"), "桥").await;
        let mom = server.login(&unique("mom"), "妈妈的手机").await;
        let localpart = mom.user_id().localpart().to_owned();
        bot.start_sync().await.expect("Bot 开始同步");
        mom.start_sync().await.expect("妈妈开始同步");

        // Bot 建和妈妈的私聊并邀请，妈妈加入；Bot 的 `m.direct` 记上这间（生产里建私聊的一方登记）。
        let room = bot
            .create_direct_chat(mom.user_id(), false)
            .await
            .expect("Bot 建私聊");
        bot.client()
            .account()
            .mark_as_dm(&room, &[mom.user_id().to_owned()])
            .await
            .expect("Bot 记私聊");
        mom.join_room(&room).await.expect("妈妈加入私聊");
        let mom_room = mom.open_room(&room).await.expect("妈妈打开私聊");
        // 消息流先挂上：发送前就位，发送后的每一条都看得到。
        let mut updates = mom_room.message_updates().await;

        let dm = MatrixDm::new(Arc::new(bot));
        eventually("Bot 的私聊标记同步进来", || {
            dm.session
                .direct_chat_rooms_with(mom.user_id())
                .now_or_never()?
                .ok()?
                .iter()
                .any(|known| known == &room)
                .then_some(())
        })
        .await;

        let event = dm
            .send_to(&localpart, "outbox-1", "周末回来吗")
            .await
            .expect("发进私聊");
        let received: String = eventually("妈妈读到那条消息", || {
            futures_util::StreamExt::next(&mut updates)
                .now_or_never()?
                .into_iter()
                .flatten()
                .find(|message: &Message| {
                    !message.is_own
                        && matches!(&message.body, MessageBody::Text(text) if text == "周末回来吗")
                })
                .and_then(|message| message.event_id.as_ref().map(ToString::to_string))
        })
        .await;
        assert_eq!(received, event);

        // 同一事务重发：服务器按事务 ID 去重，返回同一条事件，房间里只有一条。
        let again = dm
            .send_to(&localpart, "outbox-1", "周末回来吗")
            .await
            .expect("重发认回同一条");
        assert_eq!(again, event);
    }
}
