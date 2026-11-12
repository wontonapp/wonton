//! Sharing uses only the authenticated SDK session; OpenID is never cached.
use crate::{MatrixSession, OwnedUserId, Result};
use matrix_sdk::{
    RoomState,
    config::RequestConfig,
    ruma::api::client::{account::request_openid_token, state::get_state_events},
};
use std::time::Duration;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FriendSource {
    pub user_id: OwnedUserId,
    pub display_name: Option<String>,
}

/// Short-lived OpenID response. Deliberately has no Debug implementation.
pub struct OpenIdToken {
    pub access_token: String,
    pub matrix_server_name: String,
    pub expires_in: u64,
}

impl MatrixSession {
    pub async fn friend_sources(&self) -> Result<Vec<FriendSource>> {
        let mut sources = Vec::new();
        for room in self.client.joined_rooms() {
            let targets = room.direct_targets();
            if targets.len() != 1 {
                continue;
            }
            let Some(other) = targets
                .iter()
                .next()
                .and_then(|target| target.as_user_id())
                .map(ToOwned::to_owned)
            else {
                continue;
            };
            if other == room.own_user_id() {
                continue;
            }
            let response = self
                .client
                .send(get_state_events::v3::Request::new(
                    room.room_id().to_owned(),
                ))
                .with_request_config(request_config())
                .await?;
            let events = response
                .room_state
                .iter()
                .map(serde_json::to_value)
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(|error| crate::Error::Upstream(error.to_string()))?;
            if !accepted_pair(room.own_user_id().as_str(), other.as_str(), &events)
                || room.state() != RoomState::Joined
                || room.direct_targets() != targets
            {
                continue;
            }
            let display_name = events
                .iter()
                .find(|event| {
                    event.get("type").and_then(|v| v.as_str()) == Some("m.room.member")
                        && event.get("state_key").and_then(|v| v.as_str()) == Some(other.as_str())
                })
                .and_then(|event| event.pointer("/content/displayname"))
                .and_then(|value| value.as_str())
                .map(str::to_owned);
            if !sources
                .iter()
                .any(|source: &FriendSource| source.user_id == other)
            {
                sources.push(FriendSource {
                    user_id: other,
                    display_name,
                });
            }
        }
        Ok(sources)
    }

    pub async fn fresh_openid_token(&self) -> Result<OpenIdToken> {
        let token = self
            .client
            .send(request_openid_token::v3::Request::new(
                self.user_id().to_owned(),
            ))
            .with_request_config(request_config())
            .await?;
        Ok(OpenIdToken {
            access_token: token.access_token,
            matrix_server_name: token.matrix_server_name.to_string(),
            expires_in: token.expires_in.as_secs(),
        })
    }
}

fn request_config() -> RequestConfig {
    RequestConfig::new()
        .disable_retry()
        .timeout(Duration::from_secs(10))
}

fn accepted_pair(own: &str, peer: &str, events: &[serde_json::Value]) -> bool {
    let mut joined = Vec::new();
    let mut marked = false;
    for event in events {
        let kind = event.get("type").and_then(|value| value.as_str());
        let key = event.get("state_key").and_then(|value| value.as_str());
        if kind == Some("m.room.member")
            && event
                .pointer("/content/membership")
                .and_then(|value| value.as_str())
                == Some("join")
        {
            let Some(id) = key else { return false };
            joined.push(id);
        }
        if kind == Some("app.wonton.friend")
            && key == Some("")
            && event
                .get("content")
                .is_some_and(serde_json::Value::is_object)
            && event.pointer("/unsigned/redacted_because").is_none()
            && event
                .get("sender")
                .and_then(|value| value.as_str())
                .is_some_and(|id| id == own || id == peer)
        {
            marked = true;
        }
    }
    marked && joined.len() == 2 && joined.contains(&own) && joined.contains(&peer)
}

#[cfg(test)]
mod tests;
