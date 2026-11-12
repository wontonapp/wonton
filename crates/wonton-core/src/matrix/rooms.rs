//! 假 Tuwunel 的房间、成员和账号数据接口，以及 `/sync`（`fake.rs` 的另一半，按职责分开）。

use std::collections::BTreeSet;

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use serde_json::{Value, json};

use super::fake::{Fake, Reply, Room, RoomEvent, SERVER, auth, error, ok, reply};
async fn create_room(
    State(fake): State<Fake>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Reply {
    // 锁只包住算响应；bump 要再拿这把锁，得先放掉（同线程重入会死锁）。
    let response = {
        let mut state = fake.state();
        reply(auth(&state, &headers).and_then(|(user, _)| {
            if state.refuse_rooms {
                return Err(error(StatusCode::INTERNAL_SERVER_ERROR, "M_UNKNOWN"));
            }
            // 房间版本 12：建房的人不能写进 power_levels 的 users。
            if body["power_level_content_override"]["users"]
                .get(user.as_str())
                .is_some()
            {
                return Err(error(StatusCode::BAD_REQUEST, "M_INVALID_PARAM"));
            }
            let encrypted = body["initial_state"].as_array().is_some_and(|events| {
                events.iter().any(|e| {
                    e["type"] == "m.room.encryption"
                        && e["content"]["algorithm"] == "m.megolm.v1.aes-sha2"
                })
            });
            let invited: BTreeSet<String> = body["invite"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|u| u.as_str().map(str::to_owned))
                .collect();
            let invited_by = body["invite"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|u| u.as_str())
                .map(|invitee| (invitee.to_owned(), user.clone()))
                .collect();
            let id = format!("!{}:{SERVER}", state.next("room"));
            // 配对对方的核心账号由对方的收件循环接受邀请；单测里没有那台核心，替身直接替它加入，
            // 联系人房间的发件路径（发前确认对方在房）才能走通。
            let mut members = BTreeSet::from([user.clone()]);
            for invitee in invited.iter() {
                if invitee.starts_with("@wonton-core:") {
                    members.insert(invitee.clone());
                }
            }
            let room = Room {
                members,
                invited,
                left: Default::default(),
                encrypted,
                room_type: body["creation_content"]["type"].as_str().map(str::to_owned),
                creator: Some(user.clone()),
                invited_by,
            };
            state.rooms.insert(id.clone(), room);
            state.version += 1;
            Ok(ok(json!({"room_id": id})))
        }))
    };
    fake.bump();
    response
}

/// 加入、退出、邀请和撤回邀请（`kick`）。
async fn membership(
    State(fake): State<Fake>,
    headers: HeaderMap,
    Path((room, action)): Path<(String, String)>,
    body: Option<Json<Value>>,
) -> Reply {
    let response = {
        let mut state = fake.state();
        reply(auth(&state, &headers).and_then(|(user, _)| {
            // join 的开关要在拿房间可变借用之前读（同一把锁）。
            let refuse_join = state.refuse_join;
            let room_state = state
                .rooms
                .get_mut(&room)
                .ok_or_else(|| error(StatusCode::NOT_FOUND, "M_NOT_FOUND"))?;
            let target = body.and_then(|Json(b)| b["user_id"].as_str().map(str::to_owned));
            let target = target.unwrap_or_default();
            match action.as_str() {
                "join"
                    if !refuse_join
                        && (room_state.invited.remove(&user)
                            || room_state.members.contains(&user)) =>
                {
                    room_state.left.remove(&user);
                    room_state.invited_by.remove(&user);
                    room_state.members.insert(user);
                    state.version += 1;
                }
                "leave" => {
                    room_state.members.remove(&user);
                    room_state.invited.remove(&user);
                    room_state.invited_by.remove(&user);
                    room_state.left.insert(user);
                    state.version += 1;
                }
                "invite"
                    if room_state.members.contains(&user)
                        && !room_state.members.contains(&target) =>
                {
                    room_state.left.remove(&target);
                    room_state.invited.insert(target.clone());
                    room_state.invited_by.insert(target, user.clone());
                    state.version += 1;
                }
                "kick" if room_state.members.contains(&user) => {
                    room_state.invited.remove(&target);
                    room_state.invited_by.remove(&target);
                    room_state.members.remove(&target);
                    room_state.left.insert(target);
                    state.version += 1;
                }
                _ => return Err(error(StatusCode::FORBIDDEN, "M_FORBIDDEN")),
            }
            Ok(ok(json!({"room_id": room})))
        }))
    };
    fake.bump();
    response
}

/// 不在房间里时发不了。
async fn send_message(
    State(fake): State<Fake>,
    headers: HeaderMap,
    Path((room, _txn)): Path<(String, String)>,
    Json(content): Json<Value>,
) -> Reply {
    let response = {
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
            state.sent.push((room.clone(), content.clone()));
            state.events.push(RoomEvent {
                room: room.clone(),
                sender: user,
                event_type: "m.room.message".to_owned(),
                content,
            });
            state.version += 1;
            Ok(ok(json!({"event_id": event})))
        }))
    };
    fake.bump();
    response
}

/// 房间成员才能查别人的成员状态；从来没进过房间的人没有这个状态事件（404）。
async fn member_state(
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

/// 任意类型的事件（联系人房间发 `app.wonton.contact`，ADR-0050）。
async fn send_event(
    State(fake): State<Fake>,
    headers: HeaderMap,
    Path((room, event_type, _txn)): Path<(String, String, String)>,
    Json(content): Json<Value>,
) -> Reply {
    let response = {
        let mut state = fake.state();
        reply(auth(&state, &headers).and_then(|(user, _)| {
            let room_state = state
                .rooms
                .get(&room)
                .ok_or_else(|| error(StatusCode::NOT_FOUND, "M_NOT_FOUND"))?;
            if !room_state.members.contains(&user) || std::mem::take(&mut state.refuse_next_send) {
                return Err(error(StatusCode::FORBIDDEN, "M_FORBIDDEN"));
            }
            state.events.push(RoomEvent {
                room: room.clone(),
                sender: user,
                event_type,
                content,
            });
            state.version += 1;
            Ok(ok(
                json!({"event_id": format!("$event{}", state.events.len())}),
            ))
        }))
    };
    fake.bump();
    response
}

/// 房间成员的完整用户 ID（Synapse 兼容管理接口）。核心用它查远端的人接没接受邀请
/// （ADR-0050“对象是对方家里的人”）；2026-11-12 对 Tuwunel v1.9.4 实测：已登录用户可查。
async fn admin_room_members(
    State(fake): State<Fake>,
    headers: HeaderMap,
    Path(room): Path<String>,
) -> Reply {
    let state = fake.state();
    reply(auth(&state, &headers).and_then(|_| {
        let room = state
            .rooms
            .get(&room)
            .ok_or_else(|| error(StatusCode::NOT_FOUND, "M_NOT_FOUND"))?;
        let members: Vec<&str> = room.members.iter().map(String::as_str).collect();
        Ok(ok(json!({"members": members, "total": members.len()})))
    }))
}

/// `/sync`：`since` 是上次给到的版本（`s<事件数>`）。有新东西就立刻回，没有就长轮询到有或超时。
/// 邀请来自房间的邀请状态，房间重发现的 state（create + 成员）在全量同步里给出。
async fn sync(
    State(fake): State<Fake>,
    headers: HeaderMap,
    raw_query: axum::extract::RawQuery,
) -> Reply {
    let query = raw_query.0.unwrap_or_default();
    let since = query
        .split('&')
        .find_map(|pair| pair.strip_prefix("since="))
        .map(|value| value.trim_start_matches('s').parse::<usize>().unwrap_or(0));
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(25);
    let (user, _) = {
        let state = fake.state();
        match auth(&state, &headers) {
            Ok(user) => user,
            Err(reply) => return reply,
        }
    };
    let mut changes = fake.sync.1.clone();
    loop {
        let (batch, visible) = {
            let state = fake.state();
            if state.refuse_sync {
                return error(StatusCode::INTERNAL_SERVER_ERROR, "M_UNKNOWN");
            }
            // 邀请是状态：只要挂着就算“有新东西”，让收件方来 join。
            let invited: Vec<String> = state
                .rooms
                .iter()
                .filter(|(_, room)| room.invited.contains(&user))
                .map(|(id, _)| id.clone())
                .collect();
            let events: Vec<RoomEvent> = state
                .events
                .iter()
                .skip(since.unwrap_or(state.events.len()))
                .filter(|event| {
                    state
                        .rooms
                        .get(&event.room)
                        .is_some_and(|room| room.members.contains(&user))
                })
                .cloned()
                .collect();
            // 全量同步带所有在的房间；增量同步不带 join 段（rooms 的事件在下面单独给）。
            let joined: Vec<String> = if since.is_none() {
                state
                    .rooms
                    .iter()
                    .filter(|(_, room)| room.members.contains(&user))
                    .map(|(id, _)| id.clone())
                    .collect()
            } else {
                Vec::new()
            };
            (state.version, (invited, events, joined))
        };
        if visible.0.iter().any(|room| !room.is_empty())
            || !visible.1.is_empty()
            || !visible.2.is_empty()
        {
            let (invited, events, joined) = visible;
            return ok(sync_response(&fake, &user, batch, invited, events, joined));
        }
        // 没有新东西：等下一次房间变化，或者到点回空。
        let wait = changes.changed();
        if tokio::time::timeout_at(deadline, wait).await.is_err() {
            return ok(json!({"next_batch": format!("s{}", batch), "rooms": {}}));
        }
    }
}

fn sync_response(
    fake: &Fake,
    _user: &str,
    batch: u64,
    invited: Vec<String>,
    events: Vec<RoomEvent>,
    joined: Vec<String>,
) -> Value {
    let state = fake.state();
    let mut rooms = serde_json::Map::new();
    if !invited.is_empty() {
        let mut invite_rooms = serde_json::Map::new();
        for room in &invited {
            let Some(room_state) = state.rooms.get(room) else {
                continue;
            };
            let mut stripped = Vec::new();
            if let Some(creator) = &room_state.creator {
                let mut create = json!({
                    "type": "m.room.create",
                    "state_key": "",
                    "sender": creator,
                    "content": {},
                });
                if let Some(room_type) = &room_state.room_type {
                    create["content"]["type"] = json!(room_type);
                }
                stripped.push(create);
            }
            for (invitee, inviter) in &room_state.invited_by {
                stripped.push(json!({
                    "type": "m.room.member",
                    "state_key": invitee,
                    "sender": inviter,
                    "content": {"membership": "invite"},
                }));
            }
            invite_rooms.insert(room.clone(), json!({"invite_state": {"events": stripped}}));
        }
        rooms.insert("invite".to_owned(), Value::Object(invite_rooms));
    }
    let mut joined_rooms = serde_json::Map::new();
    for room in &joined {
        let Some(room_state) = state.rooms.get(room) else {
            continue;
        };
        let mut state_events = Vec::new();
        if let Some(creator) = &room_state.creator {
            let mut create = json!({
                "type": "m.room.create",
                "state_key": "",
                "sender": creator,
                "content": {},
            });
            if let Some(room_type) = &room_state.room_type {
                create["content"]["type"] = json!(room_type);
            }
            state_events.push(create);
        }
        for member in &room_state.members {
            state_events.push(json!({
                "type": "m.room.member",
                "state_key": member,
                "sender": member,
                "content": {"membership": "join"},
            }));
        }
        joined_rooms.insert(room.clone(), json!({"state": {"events": state_events}}));
    }
    if !events.is_empty() {
        for event in &events {
            let entry = joined_rooms
                .entry(event.room.clone())
                .or_insert_with(|| json!({"timeline": {"events": []}}));
            entry["timeline"]["events"].as_array_mut().expect("刚建的是数组").push(json!({
                "event_id": format!("$e{}", state.events.iter().position(|e| e == event).unwrap_or_default()),
                "sender": event.sender,
                "type": event.event_type,
                "content": event.content,
            }));
        }
    }
    if !joined_rooms.is_empty() {
        rooms.insert("join".to_owned(), Value::Object(joined_rooms));
    }
    json!({"next_batch": format!("s{batch}"), "rooms": Value::Object(rooms)})
}

async fn join(state: State<Fake>, headers: HeaderMap, Path(room): Path<String>) -> Reply {
    membership(state, headers, Path((room, "join".to_owned())), None).await
}

async fn joined_members(
    State(fake): State<Fake>,
    headers: HeaderMap,
    Path(room): Path<String>,
) -> Reply {
    let state = fake.state();
    reply(auth(&state, &headers).and_then(|(user, _)| {
        let room = state
            .rooms
            .get(&room)
            .filter(|r| r.members.contains(&user))
            .ok_or_else(|| error(StatusCode::FORBIDDEN, "M_FORBIDDEN"))?;
        let joined: serde_json::Map<String, Value> = room
            .members
            .iter()
            .map(|m| (m.clone(), json!({})))
            .collect();
        Ok(ok(json!({"joined": joined})))
    }))
}

async fn get_account_data(
    State(fake): State<Fake>,
    headers: HeaderMap,
    Path((user, event)): Path<(String, String)>,
) -> Reply {
    let state = fake.state();
    reply(auth(&state, &headers).and_then(|(caller, _)| {
        if caller != user {
            return Err(error(StatusCode::FORBIDDEN, "M_FORBIDDEN"));
        }
        match state
            .account_data
            .get(&(user.clone(), event.clone()))
            .cloned()
        {
            Some(value) => Ok(ok(value)),
            None => Err(error(StatusCode::NOT_FOUND, "M_NOT_FOUND")),
        }
    }))
}

async fn put_account_data(
    State(fake): State<Fake>,
    headers: HeaderMap,
    Path((user, event)): Path<(String, String)>,
    Json(body): Json<Value>,
) -> Reply {
    let mut state = fake.state();
    reply(auth(&state, &headers).map(|(caller, _)| {
        if caller == user {
            state.account_data.insert((user, event), body);
            ok(json!({}))
        } else {
            error(StatusCode::FORBIDDEN, "M_FORBIDDEN")
        }
    }))
}

async fn get_room_account_data(
    State(fake): State<Fake>,
    headers: HeaderMap,
    Path((user, room, event)): Path<(String, String, String)>,
) -> Reply {
    let state = fake.state();
    reply(auth(&state, &headers).and_then(|(caller, _)| {
        if caller != user {
            return Err(error(StatusCode::FORBIDDEN, "M_FORBIDDEN"));
        }
        match state
            .room_account_data
            .get(&(user.clone(), room.clone(), event.clone()))
            .cloned()
        {
            Some(value) => Ok(ok(value)),
            None => Err(error(StatusCode::NOT_FOUND, "M_NOT_FOUND")),
        }
    }))
}

async fn put_room_account_data(
    State(fake): State<Fake>,
    headers: HeaderMap,
    Path((user, room, event)): Path<(String, String, String)>,
    Json(body): Json<Value>,
) -> Reply {
    let mut state = fake.state();
    reply(auth(&state, &headers).map(|(caller, _)| {
        if caller == user {
            state.room_account_data.insert((user, room, event), body);
            ok(json!({}))
        } else {
            error(StatusCode::FORBIDDEN, "M_FORBIDDEN")
        }
    }))
}

async fn get_direct(
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

async fn put_direct(
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

/// 起一个假 Tuwunel，返回它和一份指向它的配置（签发私钥是新生成的）。
/// 房间、成员、账号数据这半边的路由（`fake.rs` 起服务时挂上）。
pub(super) fn client_router(fake: &Fake, client: &str) -> Router<()> {
    Router::new()
        .route(&format!("{client}/createRoom"), post(create_room))
        .route(&format!("{client}/join/{{room}}"), post(join))
        .route(
            &format!("{client}/rooms/{{room}}/joined_members"),
            get(joined_members),
        )
        .route(
            &format!("{client}/rooms/{{room}}/{{action}}"),
            post(membership),
        )
        .route(
            &format!("{client}/rooms/{{room}}/send/m.room.message/{{txn}}"),
            put(send_message),
        )
        .route(
            &format!("{client}/rooms/{{room}}/send/{{event_type}}/{{txn}}"),
            put(send_event),
        )
        .route(&format!("{client}/sync"), get(sync))
        .route(
            "/_synapse/admin/v1/rooms/{room}/members",
            get(admin_room_members),
        )
        .route(
            &format!("{client}/rooms/{{room}}/state/m.room.member/{{user}}"),
            get(member_state),
        )
        .route(
            &format!("{client}/user/{{user}}/account_data/m.direct"),
            get(get_direct).put(put_direct),
        )
        .route(
            &format!("{client}/user/{{user}}/account_data/{{event}}"),
            get(get_account_data).put(put_account_data),
        )
        .route(
            &format!("{client}/user/{{user}}/rooms/{{room}}/account_data/{{event}}"),
            get(get_room_account_data).put(put_room_account_data),
        )
        .with_state(fake.clone())
}
