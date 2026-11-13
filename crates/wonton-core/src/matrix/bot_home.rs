//! A creation marker lets retry recover a room whose createRoom response was lost.

use hyper::Method;
use serde_json::json;

use super::Session;
use super::api::{MatrixError, encode};

const MARKER: &str = "app.wonton.bot_home";

impl Session<'_> {
    pub(super) async fn recover_bot_home(
        &self,
        owner: &str,
    ) -> Result<Option<String>, MatrixError> {
        let rooms = self
            .json(
                Method::GET,
                "/_matrix/client/v3/joined_rooms",
                None,
                "recover bot home",
            )
            .await?;
        let mut found = None;
        for room in rooms["joined_rooms"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|room| room.as_str())
        {
            let path = format!("/_matrix/client/v3/rooms/{}/state", encode(room));
            let state = self
                .json(Method::GET, &path, None, "inspect bot home marker")
                .await?;
            let marked = state.as_array().into_iter().flatten().any(|event| {
                event["type"] == MARKER
                    && event["state_key"] == ""
                    && event["sender"] == self.user_id()
                    && event["content"]["owner"] == owner
            });
            if marked && super::bot::reusable(self, room, owner).await? {
                if found.is_some() {
                    return Err(MatrixError::Invalid(
                        "multiple bot home reservations".into(),
                    ));
                }
                found = Some(room.to_owned());
            }
        }
        Ok(found)
    }

    pub(super) async fn create_bot_home(&self, owner: &str) -> Result<String, MatrixError> {
        let body = json!({
            "preset": "trusted_private_chat",
            "room_version": "12",
            "power_level_content_override": {"users": {owner: 100}},
            "initial_state": [
                {"type": "m.room.encryption", "state_key": "", "content": {"algorithm": "m.megolm.v1.aes-sha2"}},
                {"type": MARKER, "state_key": "", "content": {"owner": owner}},
            ],
        });
        let room = self
            .json(
                Method::POST,
                "/_matrix/client/v3/createRoom",
                Some(&body),
                "create bot home",
            )
            .await?;
        room["room_id"]
            .as_str()
            .map(str::to_owned)
            .ok_or_else(|| MatrixError::Invalid("createRoom returned no room_id".into()))
    }
}
