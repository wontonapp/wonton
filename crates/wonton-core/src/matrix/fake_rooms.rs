use std::collections::BTreeMap;

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::{Value, json};

use super::{Fake, Reply, Room, SERVER, auth, error, ok, reply};

pub struct RoomState {
    pub creator: String,
    pub events: BTreeMap<(String, String), Value>,
}

pub(super) fn routes() -> Router<Fake> {
    let client = "/_matrix/client/v3";
    Router::new()
        .route(&format!("{client}/createRoom"), post(create_room))
        .route(&format!("{client}/joined_rooms"), get(joined_rooms))
        .route(&format!("{client}/rooms/{{room}}/state"), get(room_state))
        .route(
            &format!("{client}/rooms/{{room}}/state/{{event}}/"),
            get(get_state).put(put_state),
        )
}

async fn create_room(
    State(fake): State<Fake>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Reply {
    let mut state = fake.state();
    reply(auth(&state, &headers).and_then(|(user, _)| {
        if state.refuse_rooms {
            return Err(error(StatusCode::INTERNAL_SERVER_ERROR, "M_UNKNOWN"));
        }
        if body["power_level_content_override"]["users"]
            .get(user.as_str())
            .is_some()
        {
            return Err(error(StatusCode::BAD_REQUEST, "M_INVALID_PARAM"));
        }
        let mut events = BTreeMap::from([
            (
                ("m.room.create".to_owned(), String::new()),
                json!({"room_version": body["room_version"]}),
            ),
            (
                ("m.room.power_levels".to_owned(), String::new()),
                json!({"users_default": 0, "users": {}}),
            ),
        ]);
        if let Some(fields) = body["power_level_content_override"].as_object() {
            events
                .get_mut(&("m.room.power_levels".to_owned(), String::new()))
                .unwrap()
                .as_object_mut()
                .unwrap()
                .extend(fields.clone());
        }
        for event in body["initial_state"].as_array().into_iter().flatten() {
            events.insert(
                (
                    event["type"].as_str().unwrap().to_owned(),
                    event["state_key"].as_str().unwrap().to_owned(),
                ),
                event["content"].clone(),
            );
        }
        let encrypted = events
            .get(&("m.room.encryption".to_owned(), String::new()))
            .is_some_and(|content| content["algorithm"] == "m.megolm.v1.aes-sha2");
        let invited = body["invite"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|u| u.as_str().map(str::to_owned))
            .collect();
        let id = format!("!{}:{SERVER}", state.next("room"));
        state.room_state.insert(
            id.clone(),
            RoomState {
                creator: user.clone(),
                events,
            },
        );
        state.rooms.insert(
            id.clone(),
            Room {
                members: [user].into(),
                invited,
                encrypted,
                ..Room::default()
            },
        );
        Ok(ok(json!({"room_id": id})))
    }))
}

async fn joined_rooms(State(fake): State<Fake>, headers: HeaderMap) -> Reply {
    let state = fake.state();
    reply(auth(&state, &headers).map(|(user, _)| {
        let rooms: Vec<_> = state
            .rooms
            .iter()
            .filter(|(_, room)| room.members.contains(&user))
            .map(|(id, _)| id)
            .collect();
        ok(json!({"joined_rooms": rooms}))
    }))
}

async fn room_state(
    State(fake): State<Fake>,
    headers: HeaderMap,
    Path(room): Path<String>,
) -> Reply {
    let state = fake.state();
    reply(auth(&state, &headers).and_then(|(user, _)| {
        if state.refuse_room_reads {
            return Err(error(StatusCode::INTERNAL_SERVER_ERROR, "M_UNKNOWN"));
        }
        state
            .rooms
            .get(&room)
            .filter(|room| room.members.contains(&user))
            .ok_or_else(|| error(StatusCode::FORBIDDEN, "M_FORBIDDEN"))?;
        let room = &state.room_state[&room];
        let events: Vec<_> = room.events.iter().map(|((kind, key), content)| {
            json!({"type": kind, "state_key": key, "sender": room.creator, "content": content})
        }).collect();
        Ok(ok(json!(events)))
    }))
}

async fn get_state(
    State(fake): State<Fake>,
    headers: HeaderMap,
    Path((room, event)): Path<(String, String)>,
) -> Reply {
    let state = fake.state();
    reply(auth(&state, &headers).and_then(|(user, _)| {
        state
            .rooms
            .get(&room)
            .filter(|room| room.members.contains(&user))
            .ok_or_else(|| error(StatusCode::FORBIDDEN, "M_FORBIDDEN"))?;
        state.room_state[&room]
            .events
            .get(&(event, String::new()))
            .cloned()
            .map(ok)
            .ok_or_else(|| error(StatusCode::NOT_FOUND, "M_NOT_FOUND"))
    }))
}

async fn put_state(
    State(fake): State<Fake>,
    headers: HeaderMap,
    Path((room, event)): Path<(String, String)>,
    Json(content): Json<Value>,
) -> Reply {
    let mut state = fake.state();
    reply(auth(&state, &headers).and_then(|(user, _)| {
        if state.refuse_power_levels && event == "m.room.power_levels" {
            return Err(error(StatusCode::INTERNAL_SERVER_ERROR, "M_UNKNOWN"));
        }
        state
            .rooms
            .get(&room)
            .filter(|room| room.members.contains(&user))
            .ok_or_else(|| error(StatusCode::FORBIDDEN, "M_FORBIDDEN"))?;
        let room = state.room_state.get_mut(&room).unwrap();
        if event == "m.room.power_levels" && content["users"].get(&room.creator).is_some() {
            return Err(error(StatusCode::BAD_REQUEST, "M_INVALID_PARAM"));
        }
        room.events.insert((event, String::new()), content);
        Ok(ok(json!({"event_id": "$state"})))
    }))
}
