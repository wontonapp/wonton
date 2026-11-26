use hyper::Method;
use serde_json::{Map, Value, json};
use wonton_protocol::FAMILY_ROOM_EVENT;

use super::api::{MatrixError, encode};
use super::{Matrix, Session};

impl Session<'_> {
    pub(crate) async fn family_room(&self) -> Result<Option<String>, MatrixError> {
        let value = self
            .json(
                Method::GET,
                "/_matrix/client/v3/joined_rooms",
                None,
                "列出房间",
            )
            .await?;
        let rooms: Vec<String> = serde_json::from_value(value["joined_rooms"].clone())
            .map_err(|e| MatrixError::Invalid(format!("joined_rooms: {e}")))?;
        let mut family = None;
        for room in rooms {
            let path = format!("/_matrix/client/v3/rooms/{}/state", encode(&room));
            let state = self.json(Method::GET, &path, None, "读取房间状态").await?;
            let events = state
                .as_array()
                .ok_or_else(|| MatrixError::Invalid("房间状态不是事件列表".to_owned()))?;
            // 标记可由任何房间写入；只认管理账号创建的群，不能把家人拉进别人伪造的房间。
            let created = events
                .iter()
                .any(|event| event["type"] == "m.room.create" && event["sender"] == self.user_id());
            let marked = events
                .iter()
                .any(|event| event["type"] == FAMILY_ROOM_EVENT && event["state_key"] == "");
            if created && marked && family.replace(room).is_some() {
                return Err(MatrixError::Invalid(
                    "发现多个家庭群，无法确定同步目标".to_owned(),
                ));
            }
        }
        Ok(family)
    }

    pub(crate) async fn create_family_room(
        &self,
        name: &str,
        users: &Map<String, Value>,
    ) -> Result<String, MatrixError> {
        // 建群时的群名事件署核心账号的名字，排在建房之前，家人看到的才不是用户 ID。
        super::notices::name_core_account(self).await;
        // v12 的创建者有隐含最高权限，不能放进 users；人都在建房后邀请，不授予创建者身份。
        // Tuwunel v2.0.0 忽略 initial_state 中 content 为空对象的事件，标记必须有内容。
        let body = json!({
            "preset": "private_chat",
            "room_version": "12",
            "name": name,
            "power_level_content_override": {"users": users},
            "initial_state": [
                {"type": FAMILY_ROOM_EVENT, "state_key": "", "content": {"version": 1}},
            ],
        });
        let value = self
            .json(
                Method::POST,
                "/_matrix/client/v3/createRoom",
                Some(&body),
                "建家庭群",
            )
            .await?;
        value["room_id"]
            .as_str()
            .map(str::to_owned)
            .ok_or_else(|| MatrixError::Invalid("建房接口没有返回 room_id".to_owned()))
    }

    pub(crate) async fn sync_family_power_levels(
        &self,
        room: &str,
        users: &Map<String, Value>,
    ) -> Result<(), MatrixError> {
        let path = format!(
            "/_matrix/client/v3/rooms/{}/state/m.room.power_levels/",
            encode(room)
        );
        let mut content = self
            .json(Method::GET, &path, None, "读家庭群权限等级")
            .await?;
        let object = content
            .as_object_mut()
            .ok_or_else(|| MatrixError::Invalid("房间权限等级不是对象".to_owned()))?;
        if object.get("users").and_then(Value::as_object) == Some(users)
            && object.get("users_default") == Some(&json!(0))
        {
            return Ok(());
        }
        object.insert("users".to_owned(), Value::Object(users.clone()));
        object.insert("users_default".to_owned(), json!(0));
        self.json(Method::PUT, &path, Some(&content), "同步家庭群权限等级")
            .await?;
        Ok(())
    }

    pub(crate) async fn kick(&self, room: &str, user: &str) -> Result<(), MatrixError> {
        let path = format!("/_matrix/client/v3/rooms/{}/kick", encode(room));
        self.json(
            Method::POST,
            &path,
            Some(&json!({"user_id": user})),
            "移出家庭群",
        )
        .await?;
        Ok(())
    }
}

impl Matrix {
    /// 让这个人在家庭群里。`display_name` 是核心记着的名字，和他 Matrix 上的不一样时改成它（同 ADR-0030），`None` 时不动。
    pub(crate) async fn join_family_room(
        &self,
        room: &str,
        username: &str,
        display_name: Option<&str>,
    ) -> Result<(), MatrixError> {
        let user = self.user_id(username);
        let core = self.core_session();
        let membership = core.membership(room, &user).await?;
        let rename = match display_name.map(str::trim).filter(|name| !name.is_empty()) {
            Some(name) if core.display_name_of(&user).await?.as_deref() != Some(name) => Some(name),
            _ => None,
        };
        if membership.as_deref() == Some("join") && rename.is_none() {
            return Ok(());
        }
        // 先登录确保账号存在；无论邀请或加入成功与否都退出临时设备。
        let login = self.login(username, "Wonton family room").await?;
        let session = self.session(&login);
        let joined = async {
            // 先改名再加入：加入的成员事件带着名字，群里不多一条改名。
            if let Some(name) = rename {
                session.set_display_name(name).await?;
            }
            if membership.as_deref() != Some("join") {
                if membership.as_deref() != Some("invite") {
                    core.invite(room, &user).await?;
                }
                session.join(room).await?;
            }
            Ok::<_, MatrixError>(())
        }
        .await;
        let logout = session.logout().await;
        if let Err(error) = &logout {
            tracing::error!(%user, %error, "Failed to log out family room session");
        }
        joined.and(logout)
    }
}
