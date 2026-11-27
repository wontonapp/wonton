//! 假 Tuwunel 的房间接口：建房、加入、成员、房间状态、发消息、别名解析和 `m.direct`。

use std::collections::BTreeMap;

use axum::body::{Body, Bytes};
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::{Value, json};

use super::http::{Reply, auth, error, ok, reply};
use super::state::{ADMIN_ROOM_ALIAS, Fake, Room, SERVER};

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
) -> Response {
    let mut state = fake.state();
    let dropped = std::mem::take(&mut state.drop_next_room_create);
    let result = reply(auth(&state, &headers).and_then(|(user, _)| {
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
            if event["content"]
                .as_object()
                .is_some_and(|content| content.is_empty())
            {
                continue;
            }
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
    }));
    if dropped {
        // 状态已经改完，响应体在发完头之后断掉：客户端收到的是传输错误，不是拒绝。
        let stream = futures_util::stream::once(async {
            Err::<Bytes, std::io::Error>(std::io::Error::new(
                std::io::ErrorKind::ConnectionReset,
                "createRoom 的响应丢了",
            ))
        });
        return (StatusCode::OK, Body::from_stream(stream)).into_response();
    }
    result.into_response()
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

/// 房间成员用状态接口改别人的成员事件：`invite` 记成邀请，其余（`ban`、`leave`）按移出房间。
pub(super) async fn put_member_state(
    State(fake): State<Fake>,
    headers: HeaderMap,
    Path((room, target)): Path<(String, String)>,
    Json(content): Json<Value>,
) -> Reply {
    let mut state = fake.state();
    reply(auth(&state, &headers).and_then(|(user, _)| {
        let room = state
            .rooms
            .get_mut(&room)
            .filter(|room| room.members.contains(&user))
            .ok_or_else(|| error(StatusCode::FORBIDDEN, "M_FORBIDDEN"))?;
        if content["membership"] == "invite" {
            room.left.remove(&target);
            room.invited.insert(target);
        } else {
            room.invited.remove(&target);
            room.members.remove(&target);
            room.left.insert(target);
        }
        Ok(ok(json!({"event_id": "$member"})))
    }))
}

/// 加入、退出、邀请和撤回邀请（`kick`）。
pub(super) async fn membership(
    State(fake): State<Fake>,
    headers: HeaderMap,
    Path((room, action)): Path<(String, String)>,
    body: Option<Json<Value>>,
) -> Reply {
    let mut state = fake.state();
    reply(auth(&state, &headers).and_then(|(user, _)| {
        if action == "kick" && state.refuse_kicks
            || action == "invite" && state.refuse_invites
            || action == "join" && state.refuse_joins
        {
            return Err(error(StatusCode::INTERNAL_SERVER_ERROR, "M_UNKNOWN"));
        }
        let room_state = state
            .rooms
            .get_mut(&room)
            .ok_or_else(|| error(StatusCode::NOT_FOUND, "M_NOT_FOUND"))?;
        let target = body.and_then(|Json(b)| b["user_id"].as_str().map(str::to_owned));
        let target = target.unwrap_or_default();
        match action.as_str() {
            "join" if room_state.invited.remove(&user) || room_state.members.contains(&user) => {
                room_state.left.remove(&user);
                room_state.members.insert(user);
            }
            "leave" => {
                room_state.members.remove(&user);
                room_state.left.insert(user);
            }
            "invite"
                if room_state.members.contains(&user) && !room_state.members.contains(&target) =>
            {
                room_state.left.remove(&target);
                room_state.invited.insert(target);
            }
            "kick" if room_state.members.contains(&user) => {
                room_state.invited.remove(&target);
                room_state.members.remove(&target);
                room_state.left.insert(target);
            }
            _ => return Err(error(StatusCode::FORBIDDEN, "M_FORBIDDEN")),
        }
        Ok(ok(json!({"room_id": room})))
    }))
}

/// 房间别名解析：只有管理房间的别名。
pub(super) async fn directory_room(State(fake): State<Fake>, Path(alias): Path<String>) -> Reply {
    let state = fake.state();
    if alias == ADMIN_ROOM_ALIAS {
        ok(json!({"room_id": state.admin_room, "servers": [SERVER]}))
    } else {
        error(StatusCode::NOT_FOUND, "M_NOT_FOUND")
    }
}

/// 不在房间里时发不了。
pub(super) async fn send_message(
    State(fake): State<Fake>,
    headers: HeaderMap,
    Path((room, _txn)): Path<(String, String)>,
    Json(content): Json<Value>,
) -> Reply {
    let mut state = fake.state();
    reply(auth(&state, &headers).and_then(|(user, _)| {
        let room_state = state
            .rooms
            .get(&room)
            .ok_or_else(|| error(StatusCode::NOT_FOUND, "M_NOT_FOUND"))?;
        if !room_state.members.contains(&user) || std::mem::take(&mut state.refuse_next_send) {
            return Err(error(StatusCode::FORBIDDEN, "M_FORBIDDEN"));
        }
        let event = format!("$event{}", state.sent.len());
        state.sent.push((room.clone(), content));
        Ok(ok(json!({"event_id": event})))
    }))
}

/// 房间成员才能查别人的成员状态；从来没进过房间的人没有这个状态事件（404）。
pub(super) async fn member_state(
    State(fake): State<Fake>,
    headers: HeaderMap,
    Path((room, target)): Path<(String, String)>,
) -> Reply {
    let state = fake.state();
    reply(auth(&state, &headers).and_then(|(user, _)| {
        let room = state
            .rooms
            .get(&room)
            .filter(|r| r.members.contains(&user))
            .ok_or_else(|| error(StatusCode::FORBIDDEN, "M_FORBIDDEN"))?;
        let membership = if room.members.contains(&target) {
            "join"
        } else if room.invited.contains(&target) {
            "invite"
        } else if room.left.contains(&target) {
            "leave"
        } else {
            return Err(error(StatusCode::NOT_FOUND, "M_NOT_FOUND"));
        };
        Ok(ok(json!({"membership": membership})))
    }))
}

pub(super) async fn join(
    state: State<Fake>,
    headers: HeaderMap,
    Path(room): Path<String>,
) -> Reply {
    membership(state, headers, Path((room, "join".to_owned())), None).await
}

pub(super) async fn joined_members(
    State(fake): State<Fake>,
    headers: HeaderMap,
    Path(room): Path<String>,
) -> Reply {
    let state = fake.state();
    if state.refuse_room_reads {
        return reply(Err(error(StatusCode::INTERNAL_SERVER_ERROR, "M_UNKNOWN")));
    }
    reply(auth(&state, &headers).and_then(|(user, _)| {
        if state.broken_rooms.contains(&room) {
            return Err(error(StatusCode::INTERNAL_SERVER_ERROR, "M_UNKNOWN"));
        }
        let room = state
            .rooms
            .get(&room)
            .ok_or_else(|| error(StatusCode::NOT_FOUND, "M_NOT_FOUND"))?;
        if !room.members.contains(&user) {
            return Err(error(StatusCode::FORBIDDEN, "M_FORBIDDEN"));
        }
        let joined: serde_json::Map<String, Value> = room
            .members
            .iter()
            .map(|m| (m.clone(), json!({})))
            .collect();
        Ok(ok(json!({"joined": joined})))
    }))
}

pub(super) async fn get_direct(
    State(fake): State<Fake>,
    headers: HeaderMap,
    Path(user): Path<String>,
) -> Reply {
    let state = fake.state();
    reply(auth(&state, &headers).and_then(|(caller, _)| {
        let direct = state.users.get(&user).and_then(|u| u.direct.clone());
        match direct {
            Some(direct) if caller == user => Ok(ok(direct)),
            _ if caller != user => Err(error(StatusCode::FORBIDDEN, "M_FORBIDDEN")),
            _ => Err(error(StatusCode::NOT_FOUND, "M_NOT_FOUND")),
        }
    }))
}

pub(super) async fn put_direct(
    State(fake): State<Fake>,
    headers: HeaderMap,
    Path(user): Path<String>,
    Json(body): Json<Value>,
) -> Reply {
    let mut state = fake.state();
    reply(auth(&state, &headers).and_then(|(caller, _)| {
        if caller != user {
            return Err(error(StatusCode::FORBIDDEN, "M_FORBIDDEN"));
        }
        state.users.entry(user).or_default().direct = Some(body);
        Ok(ok(json!({})))
    }))
}
