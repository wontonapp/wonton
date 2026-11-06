//! 服务器发来的同步把刚加入的房间标成“受邀”：时间线里有加入事件，`required_state` 里本人却还是邀请。
//! Tuwunel 先让新事件对同步可见、再更新房间当前状态，这中间的一次同步就是这样（CI 上实测）。模拟的核心改写
//! 加入之后的第一个这样的同步应答，确定地造出这种情况。

use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use wonton_client::{Action, RoomMembership};
use wonton_matrix::OwnedUserId;

use super::support::{Home, OWNER, Phone};

/// 加入后同步一直标着“受邀”时，客户端重新订阅这个房间，服务器按当前状态重发，房间变成已加入。
#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn a_join_the_server_reports_as_invited_still_ends_joined() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let home = Home::start(&[phone.device_id()]).await;
    phone.connect(&home);
    phone.signed_in().await;
    let bot = home.bot("hermes", "Hermes").await;
    let owner: OwnedUserId = OWNER.parse().unwrap();
    let room_id = bot
        .create_direct_chat(&owner, false)
        .await
        .expect("Bot 发起私聊");
    phone
        .wait_chat("收到 Bot 的邀请", |view| {
            view.rooms
                .iter()
                .find(|row| {
                    row.room_id == room_id.as_str()
                        && row.membership == RoomMembership::Invited
                        && row.is_direct
                })
                .map(|_| ())
        })
        .await;
    home.report_join_as_invited(StaleJoin {
        room_id: room_id.to_string(),
        user_id: OWNER.to_owned(),
    });
    phone
        .dispatch(Action::JoinRoom {
            room_id: room_id.to_string(),
        })
        .unwrap();
    phone
        .wait_chat("加入后变成已加入", |view| {
            view.rooms
                .iter()
                .find(|row| {
                    row.room_id == room_id.as_str() && row.membership == RoomMembership::Joined
                })
                .map(|_| ())
        })
        .await;
    assert!(
        home.join_was_reported_as_invited(),
        "加入后的同步应答被改写过"
    );
}

/// 要改写的房间和本人。
#[derive(Clone)]
pub struct StaleJoin {
    pub room_id: String,
    pub user_id: String,
}

/// 只转一个请求：让 Tuwunel 回完就断开、不压缩，读完整个应答。是同步应答、里面有 `stale` 那个房间的加入事件，
/// 就把 `required_state` 里本人的成员事件换成加入前的邀请。返回是否改写了。
pub async fn forward_one(
    mut tcp: TcpStream,
    tuwunel: &str,
    head: &[u8],
    stale: &StaleJoin,
) -> bool {
    let head = String::from_utf8_lossy(head);
    let mut request: String = head
        .split("\r\n")
        .filter(|line| {
            let lower = line.to_ascii_lowercase();
            !line.is_empty()
                && !lower.starts_with("accept-encoding:")
                && !lower.starts_with("connection:")
        })
        .map(|line| format!("{line}\r\n"))
        .collect();
    request.push_str("connection: close\r\n\r\n");
    let mut body = vec![0u8; header(&head, "content-length").map_or(0, |v| v.parse().unwrap())];
    if tcp.read_exact(&mut body).await.is_err() {
        return false;
    }
    let mut upstream = TcpStream::connect(tuwunel).await.unwrap();
    upstream.write_all(request.as_bytes()).await.unwrap();
    upstream.write_all(&body).await.unwrap();
    let mut response = Vec::new();
    let _ = upstream.read_to_end(&mut response).await;
    let is_sync = head
        .lines()
        .next()
        .is_some_and(|line| line.contains("/sync"));
    let rewritten = is_sync.then(|| rewrite(&response, stale)).flatten();
    let changed = rewritten.is_some();
    let _ = tcp.write_all(&rewritten.unwrap_or(response)).await;
    let _ = tcp.shutdown().await;
    changed
}

fn header<'a>(head: &'a str, name: &str) -> Option<&'a str> {
    head.lines().find_map(|line| {
        let (key, value) = line.split_once(':')?;
        key.eq_ignore_ascii_case(name).then(|| value.trim())
    })
}

fn rewrite(response: &[u8], stale: &StaleJoin) -> Option<Vec<u8>> {
    let split = response.windows(4).position(|w| w == b"\r\n\r\n")?;
    let head = String::from_utf8_lossy(&response[..split]).into_owned();
    let raw = &response[split + 4..];
    let body = if header(&head, "transfer-encoding").is_some_and(|v| v.contains("chunked")) {
        dechunk(raw)?
    } else {
        raw.to_vec()
    };
    let mut json: Value = serde_json::from_slice(&body).ok()?;
    if !report_as_invited(&mut json, stale) {
        return None;
    }
    let body = serde_json::to_vec(&json).unwrap();
    let mut out: String = head
        .split("\r\n")
        .filter(|line| {
            let lower = line.to_ascii_lowercase();
            !lower.starts_with("content-length:") && !lower.starts_with("transfer-encoding:")
        })
        .map(|line| format!("{line}\r\n"))
        .collect();
    out.push_str(&format!("content-length: {}\r\n\r\n", body.len()));
    let mut out = out.into_bytes();
    out.extend(body);
    Some(out)
}

fn dechunk(mut raw: &[u8]) -> Option<Vec<u8>> {
    let mut body = Vec::new();
    loop {
        let line_end = raw.windows(2).position(|w| w == b"\r\n")?;
        let size = std::str::from_utf8(&raw[..line_end])
            .ok()?
            .split(';')
            .next()?;
        let size = usize::from_str_radix(size.trim(), 16).ok()?;
        raw = &raw[line_end + 2..];
        if size == 0 {
            return Some(body);
        }
        body.extend_from_slice(raw.get(..size)?);
        raw = raw.get(size + 2..)?;
    }
}

/// 时间线里有本人的加入事件（加入前是邀请）时，`required_state` 里本人的成员事件换成那次邀请。
fn report_as_invited(json: &mut Value, stale: &StaleJoin) -> bool {
    let Some(room) = json
        .get_mut("rooms")
        .and_then(|rooms| rooms.get_mut(&stale.room_id))
    else {
        return false;
    };
    let is_own_member = |event: &Value| {
        event["type"] == "m.room.member" && event["state_key"] == stale.user_id.as_str()
    };
    let Some(join) = room["timeline"].as_array().and_then(|timeline| {
        timeline.iter().find(|event| {
            is_own_member(event)
                && event["content"]["membership"] == "join"
                && event["unsigned"]["prev_content"]["membership"] == "invite"
        })
    }) else {
        return false;
    };
    let mut invite = join.clone();
    invite["content"] = join["unsigned"]["prev_content"].clone();
    invite["event_id"] = Value::from("$stale-invite");
    invite.as_object_mut().unwrap().remove("unsigned");
    let required = room
        .as_object_mut()
        .unwrap()
        .entry("required_state")
        .or_insert_with(|| Value::Array(Vec::new()));
    let required = required.as_array_mut().unwrap();
    required.retain(|event| !is_own_member(event));
    required.push(invite);
    true
}
