//! 核心发给主人的通知房间“Wonton”（ADR-0016）：核心账号和主人的私聊，确认卡片发在这里。核心没有端到端加密
//! 设备，这个房间不加密。
//!
//! 房间 ID 记在核心的状态目录里，核心和主人都还在房间里就一直沿用，否则新建一个，并退出旧的、从 `m.direct` 里去掉，主人退出
//! 多少次都只留一个。每次发之前都核对：发进一个主人已经退出的房间也会“成功”，其实谁都收不到。

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::{Value, json};
use tokio::sync::Mutex;

use super::api::MatrixError;
use super::{Matrix, Session};
use crate::plugin::languages::Languages;

/// 通知房间的名字，也是核心账号的显示名：通知的标题用发送者的显示名，没设时主人看到的是 `wonton-core`。
const ROOM_NAME: &str = "Wonton";

/// 发出去的一条消息在哪。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Posted {
    pub room: String,
    pub event: String,
}

/// 编辑的结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Edited {
    /// 在原房间里改好了。
    InPlace,
    /// 原房间不能用了，在当前的房间里重发了一条完整的。
    Reposted(Posted),
}

pub struct NoticeRoom {
    matrix: Arc<Matrix>,
    /// 主人的用户名（localpart）。
    owner: String,
    /// 记房间 ID 的文件。
    saved: PathBuf,
    room: Mutex<Option<String>>,
    /// 主人的语言：卡片和踢出理由按它写。每次都读，主人换了语言下一条就是新的。
    pub(crate) languages: Arc<dyn Fn() -> Languages + Send + Sync>,
}

impl NoticeRoom {
    pub fn new(
        matrix: Arc<Matrix>,
        owner: &str,
        saved: PathBuf,
        languages: Arc<dyn Fn() -> Languages + Send + Sync>,
    ) -> Self {
        Self {
            matrix,
            owner: owner.to_owned(),
            saved,
            room: Mutex::new(None),
            languages,
        }
    }

    /// 以核心账号发一条消息。核对之后才被踢出去的话再试一次，那时会换一个房间。
    pub async fn send(&self, content: &Value) -> Result<Posted, MatrixError> {
        self.send_with_txn(content, &crate::secret::random_token())
            .await
    }

    pub(crate) async fn send_with_txn(
        &self,
        content: &Value,
        transaction: &str,
    ) -> Result<Posted, MatrixError> {
        let room = self.room().await?;
        let core = self.matrix.core_session();
        let (room, event) = match core
            .send_message_with_txn(&room, content, transaction)
            .await
        {
            Err(e) if gone(&e) => {
                let room = self.room().await?;
                let event = core
                    .send_message_with_txn(&room, content, transaction)
                    .await?;
                (room, event)
            }
            sent => (room, sent?),
        };
        Ok(Posted { room, event })
    }

    /// 以核心账号把发过的消息编辑成 `content`（`m.replace`）。Matrix 客户端不理跨房间的编辑，所以只在原房间里改；原房间不能用了，
    /// 就在当前的房间里重发一条完整的。这条路径不建房间：当前也没有能用的房间时如实报错，下一次 [`Self::send`] 才建。
    pub async fn edit(&self, posted: &Posted, content: &Value) -> Result<Edited, MatrixError> {
        let core = self.matrix.core_session();
        let owner = self.matrix.user_id(&self.owner);
        if both_joined(&core, &posted.room, &owner).await? {
            let mut edit = content.clone();
            if let Some(body) = content["body"].as_str() {
                edit["body"] = json!(format!("* {body}"));
            }
            edit["m.new_content"] = content.clone();
            edit["m.relates_to"] = json!({"rel_type": "m.replace", "event_id": posted.event});
            match core.send_message(&posted.room, &edit).await {
                Err(e) if gone(&e) => {}
                sent => return sent.map(|_| Edited::InPlace),
            }
        }
        let current = known(&self.saved, &*self.room.lock().await);
        let room = match current.filter(|room| *room != posted.room) {
            Some(room) if both_joined(&core, &room, &owner).await? => room,
            _ => {
                return Err(MatrixError::Invalid(format!(
                    "原来的通知房间 {} 不能用了，也没有新的房间可以重发",
                    posted.room
                )));
            }
        };
        let event = core.send_message(&room, content).await?;
        Ok(Edited::Reposted(Posted { room, event }))
    }

    /// 通知房间的 ID：记下的还能用就沿用，否则新建，再收拾旧的。
    async fn room(&self) -> Result<String, MatrixError> {
        let core = self.matrix.core_session();
        let owner = self.matrix.user_id(&self.owner);
        let mut room = self.room.lock().await;
        // 排在建房之前：新房间里不会多出一条改名记录。
        if room.is_none() {
            name_core_account(&core).await;
        }
        let old = known(&self.saved, &room);
        let id = match &old {
            Some(id) if both_joined(&core, id, &owner).await? => id.clone(),
            _ => {
                let new = self.create(&core, &owner).await?;
                if let Some(old) = &old {
                    core.retire_direct(&owner, old).await;
                }
                new
            }
        };
        *room = Some(id.clone());
        Ok(id)
    }

    async fn create(&self, core: &Session<'_>, owner: &str) -> Result<String, MatrixError> {
        let id = core.create_notice_room(ROOM_NAME, owner).await?;
        let handed_over = async {
            core.add_direct(owner, &id).await?;
            self.matrix
                .owner_joins(&self.owner, core.user_id(), &id)
                .await
        }
        .await;
        if let Err(e) = handed_over {
            if let Err(cleanup) = core.abandon(&id, owner, &(self.languages)()).await {
                tracing::error!(room = %id, error = %cleanup, "撤回没建好的通知房间失败");
            }
            return Err(e);
        }
        std::fs::write(&self.saved, &id)
            .map_err(|e| MatrixError::Invalid(format!("记下通知房间失败：{e}")))?;
        tracing::info!(room = %id, "建好了和主人的通知房间");
        Ok(id)
    }
}

/// 给核心账号设显示名。已经是这个名字就不写：改名会在核心的每个房间里留一条记录。名字只影响通知的标题，设不上不能挡住
/// 主人要确认的卡片，所以只记错误日志。
async fn name_core_account(core: &Session<'_>) {
    let named = async {
        if core.display_name().await?.as_deref() != Some(ROOM_NAME) {
            core.set_display_name(ROOM_NAME).await?;
        }
        Ok::<_, MatrixError>(())
    }
    .await;
    if let Err(e) = named {
        tracing::error!(error = %e, "设置核心账号的显示名失败");
    }
}

/// 记下的房间：内存里的，否则状态目录里的。
fn known(saved: &Path, room: &Option<String>) -> Option<String> {
    room.clone()
        .or_else(|| {
            std::fs::read_to_string(saved)
                .ok()
                .map(|s| s.trim().to_owned())
        })
        .filter(|id| !id.is_empty())
}

/// 这个房间不能再用了。Tuwunel v1.9.4 实测（用共享密钥注册的账号直接请求）：不在房间里或者房间不存在时，`joined_members` 都回
/// `M_FORBIDDEN`；退出以后再发消息也是。`M_NOT_FOUND` 是规范里别的服务器可能回的。
fn gone(error: &MatrixError) -> bool {
    matches!(error.errcode(), "M_FORBIDDEN" | "M_NOT_FOUND")
}

/// 核心和主人是不是都还在房间里。
async fn both_joined(core: &Session<'_>, room: &str, owner: &str) -> Result<bool, MatrixError> {
    match core.joined_members(room).await {
        Ok(members) => Ok(members.iter().any(|m| m == owner)),
        Err(e) if gone(&e) => Ok(false),
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::super::fake::start;
    use super::*;

    fn test_room(matrix: &Arc<Matrix>, saved: PathBuf) -> NoticeRoom {
        NoticeRoom::new(matrix.clone(), "owner", saved, Arc::new(Languages::default))
    }

    use crate::http::HttpClient;

    async fn notices(dir: &tempfile::TempDir) -> (super::super::fake::Fake, NoticeRoom) {
        let (fake, config) = start().await;
        let matrix = Matrix::connect(&config, dir.path(), HttpClient::new())
            .await
            .unwrap();
        let saved = dir.path().join("notice-room");
        (fake, test_room(&Arc::new(matrix), saved))
    }

    fn text() -> Value {
        json!({"msgtype": "m.text", "body": "好"})
    }

    /// 消息发到了哪个房间，这个房间现在有谁。
    fn last_sent(fake: &super::super::fake::Fake) -> (String, Vec<String>) {
        let state = fake.state();
        let (room, _) = state.sent.last().expect("没有发出消息").clone();
        let members = state.rooms[&room].members.iter().cloned().collect();
        (room, members)
    }

    const OWNER: &str = "@owner:test";

    #[tokio::test]
    async fn a_message_reaches_the_owner_after_they_leave_the_room() {
        let dir = tempfile::tempdir().unwrap();
        let (fake, notices) = notices(&dir).await;
        notices.send(&text()).await.unwrap();
        let (first, members) = last_sent(&fake);
        assert!(members.iter().any(|m| m == OWNER));
        notices.send(&text()).await.unwrap();
        assert_eq!(last_sent(&fake).0, first, "都在就沿用");

        fake.state()
            .rooms
            .get_mut(&first)
            .unwrap()
            .members
            .remove(OWNER);
        notices.send(&text()).await.unwrap();
        let (second, members) = last_sent(&fake);
        assert_ne!(second, first);
        assert!(members.iter().any(|m| m == OWNER));
    }

    /// 核心账号现在的显示名。
    fn core_name(fake: &super::super::fake::Fake, notices: &NoticeRoom) -> Option<String> {
        let core = notices.matrix.core_session();
        fake.state().user(core.user_id()).display_name.clone()
    }

    #[tokio::test]
    async fn notifications_from_the_core_account_are_titled_wonton() {
        let dir = tempfile::tempdir().unwrap();
        let (fake, notices) = notices(&dir).await;
        notices.send(&text()).await.unwrap();
        assert_eq!(core_name(&fake, &notices).as_deref(), Some(ROOM_NAME));
    }

    #[tokio::test]
    async fn a_core_that_already_has_a_room_gets_its_name_after_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        let (fake, notices) = notices(&dir).await;
        notices.send(&text()).await.unwrap();
        let (room, _) = last_sent(&fake);
        let core = notices.matrix.core_session().user_id().to_owned();
        fake.state().users.get_mut(&core).unwrap().display_name = None;

        let restarted = test_room(&notices.matrix, notices.saved.clone());
        restarted.send(&text()).await.unwrap();
        assert_eq!(last_sent(&fake).0, room, "沿用原来的房间");
        assert_eq!(core_name(&fake, &restarted).as_deref(), Some(ROOM_NAME));
    }

    #[tokio::test]
    async fn a_core_with_the_right_name_is_not_renamed_again() {
        let dir = tempfile::tempdir().unwrap();
        let (fake, notices) = notices(&dir).await;
        notices.send(&text()).await.unwrap();
        let restarted = test_room(&notices.matrix, notices.saved.clone());
        restarted.send(&text()).await.unwrap();
        assert_eq!(fake.state().display_name_writes, 1);
    }

    #[tokio::test]
    async fn a_name_that_cannot_be_set_does_not_hold_back_the_card() {
        let dir = tempfile::tempdir().unwrap();
        let (fake, notices) = notices(&dir).await;
        fake.state().refuse_profile = true;
        notices.send(&text()).await.unwrap();
        assert_eq!(fake.state().sent.len(), 1);
        assert_eq!(core_name(&fake, &notices), None);
    }

    #[tokio::test]
    async fn a_core_account_thrown_out_of_the_room_gets_a_new_one() {
        let dir = tempfile::tempdir().unwrap();
        let (fake, notices) = notices(&dir).await;
        notices.send(&text()).await.unwrap();
        let (first, _) = last_sent(&fake);

        // 核对之后才被拒绝：再试一次就发出去了，不报错。
        fake.state().refuse_next_send = true;
        notices.send(&text()).await.unwrap();

        fake.state()
            .rooms
            .get_mut(&first)
            .unwrap()
            .members
            .remove(CORE);
        notices.send(&text()).await.unwrap();
        let (second, members) = last_sent(&fake);
        assert_ne!(second, first);
        assert!(members.iter().any(|m| m == OWNER));
    }

    fn owner_leaves(fake: &super::super::fake::Fake, room: &str) {
        let mut state = fake.state();
        let room = state.rooms.get_mut(room).unwrap();
        room.members.remove(OWNER);
        room.left.insert(OWNER.to_owned());
    }

    const CORE: &str = "@wonton-core:test";

    #[tokio::test]
    async fn an_edit_stays_in_the_room_of_the_message() {
        let dir = tempfile::tempdir().unwrap();
        let (fake, notices) = notices(&dir).await;
        let posted = notices.send(&text()).await.unwrap();
        assert_eq!(
            notices.edit(&posted, &text()).await.unwrap(),
            Edited::InPlace
        );
        let state = fake.state();
        let (room, edit) = state.sent.last().unwrap();
        assert_eq!(*room, posted.room);
        assert_eq!(edit["m.relates_to"]["event_id"], json!(posted.event));
    }

    /// 跨房间的 `m.replace` 客户端不理：报成功却谁也看不到。
    #[tokio::test]
    async fn an_edit_after_the_room_changed_reposts_the_whole_message() {
        let dir = tempfile::tempdir().unwrap();
        let (fake, notices) = notices(&dir).await;
        let first = notices.send(&text()).await.unwrap();
        owner_leaves(&fake, &first.room);
        let second = notices.send(&text()).await.unwrap();
        assert_ne!(second.room, first.room);

        let Edited::Reposted(reposted) = notices.edit(&first, &text()).await.unwrap() else {
            panic!("原房间不能用了");
        };
        assert_eq!(reposted.room, second.room);
        {
            let state = fake.state();
            let (room, content) = state.sent.last().unwrap();
            assert_eq!(*room, second.room);
            assert!(content.get("m.relates_to").is_none(), "要是一条完整的消息");
        }

        // 之后的编辑跟着新的那条。
        assert_eq!(
            notices.edit(&reposted, &text()).await.unwrap(),
            Edited::InPlace
        );
        let state = fake.state();
        let (room, edit) = state.sent.last().unwrap();
        assert_eq!(*room, second.room);
        assert_eq!(edit["m.relates_to"]["event_id"], json!(reposted.event));
    }

    #[tokio::test]
    async fn an_edit_with_no_room_left_says_so_and_creates_none() {
        let dir = tempfile::tempdir().unwrap();
        let (fake, notices) = notices(&dir).await;
        let posted = notices.send(&text()).await.unwrap();
        owner_leaves(&fake, &posted.room);
        let (rooms, sent) = {
            let state = fake.state();
            (state.rooms.len(), state.sent.len())
        };

        assert!(notices.edit(&posted, &text()).await.is_err());
        let state = fake.state();
        assert_eq!(state.rooms.len(), rooms, "编辑不建房间");
        assert_eq!(state.sent.len(), sent);
    }

    /// 主人退出一次，核心也退出旧房间、从 `m.direct` 去掉，不越积越多。
    #[tokio::test]
    async fn a_new_room_clears_away_the_old_one() {
        let dir = tempfile::tempdir().unwrap();
        let (fake, notices) = notices(&dir).await;
        let first = notices.send(&text()).await.unwrap();
        owner_leaves(&fake, &first.room);
        let second = notices.send(&text()).await.unwrap();

        let state = fake.state();
        assert!(!state.rooms[&first.room].members.contains(CORE));
        assert_eq!(
            state.users[CORE].direct.as_ref().unwrap()[OWNER],
            json!([second.room])
        );
    }

    /// 旧房间收拾不掉不耽误发消息：核心早被踢出去时退出会被拒绝，`m.direct` 照样去掉。
    #[tokio::test]
    async fn an_old_room_that_cannot_be_left_is_still_forgotten() {
        let dir = tempfile::tempdir().unwrap();
        let (fake, notices) = notices(&dir).await;
        let first = notices.send(&text()).await.unwrap();
        fake.state().rooms.remove(&first.room);
        let second = notices.send(&text()).await.unwrap();

        let state = fake.state();
        assert_eq!(
            state.users[CORE].direct.as_ref().unwrap()[OWNER],
            json!([second.room])
        );
    }
}
