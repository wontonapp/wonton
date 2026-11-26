//! 服务器发来的同步把刚加入的房间标成“受邀”：时间线里有加入事件，`required_state` 里本人却还是邀请。
//! Tuwunel 先让新事件对同步可见、再更新房间当前状态，这中间的一次同步就是这样（CI 上实测）。模拟核心保持
//! 这种错标，测试看到改写送达后清除，让客户端通过重新订阅拿回真实状态。
//!
//! 加入事件只在一个应答里送达一次：改写要等到它的那条应答，会话里别的连接或者重启前的请求先把它吃掉，
//! 之后再怎么重启都不会带回它（CI 上因此连红）。见过一次加入事件之后（哪怕那条连接被 SDK 丢掉），应答里
//! 已经没有了它时也按房间成员现场造一条邀请继续改写。还没见过加入事件的应答（加入前就在飞的 `/sync`）原样
//! 放过，否则改写会提前记账，测试随即清掉，加入应答报已加入，`until_joined` 的重新订阅整段被跳过。

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use wonton_client::{Action, RoomMembership};
use wonton_matrix::OwnedUserId;

use super::support::{Home, OWNER, Phone, WAIT, is_room_list};

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
    // 先建房、后邀请，好把改写挂在邀请之前：邀请本身的应答就是一条加入前的、带着这个房间的同步应答。
    // 它的内容其实没变，提前记账的改写会让测试马上清掉，加入应答直接报已加入，`until_joined` 的重新订阅整段被跳过。
    let room_id = bot
        .create_empty_group_room("群聊")
        .await
        .expect("Bot 建群聊");
    home.report_join_as_invited(StaleJoin::new(room_id.to_string(), OWNER));
    bot.invite_user(&room_id, &owner)
        .await
        .expect("Bot 邀请主人");
    phone
        .wait_chat("收到邀请", |view| {
            view.rooms
                .iter()
                .find(|row| {
                    row.room_id == room_id.as_str() && row.membership == RoomMembership::Invited
                })
                .map(|_| ())
        })
        .await;
    phone.wait_ready().await;
    phone
        .dispatch(Action::Chat {
            action: wonton_protocol::ChatAction::JoinRoom {
                room_id: room_id.to_string(),
            },
        })
        .unwrap();
    // 改写持续到显式清除（会话里多条同步连接都会带回房间状态）。
    // 第一次改写只可能落在带着加入事件的那条应答上：它把加入抹掉，客户端等不来已加入，只能重新订阅。
    let deadline = Instant::now() + WAIT;
    while !home.join_was_reported_as_invited() {
        assert!(
            Instant::now() < deadline,
            "joined sync response was never rewritten"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    home.clear_join_report_as_invited();
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
    // 房间没有打开过，只有 `until_joined` 的那次强制订阅会在房间列表请求里带上它；提前记账跳过重新订阅时
    // 一条请求都没有，`wait_chat` 照样能绿。
    assert!(
        home.sync_requests().iter().any(|request| {
            is_room_list(&request["conn_id"])
                && request["room_subscriptions"][room_id.as_str()].is_object()
        }),
        "没有重新订阅这个房间"
    );
}

/// 要改写的房间和本人。
#[derive(Clone)]
pub struct StaleJoin {
    pub room_id: String,
    pub user_id: String,
    /// 已经有一条应答带着本人的加入事件，不管那条连接最后有没有送达。
    seen_join: Arc<AtomicBool>,
}

impl StaleJoin {
    pub fn new(room_id: impl Into<String>, user_id: impl Into<String>) -> Self {
        Self {
            room_id: room_id.into(),
            user_id: user_id.into(),
            seen_join: Arc::new(AtomicBool::new(false)),
        }
    }
}

/// 只转一个请求：让 Tuwunel 回完就断开、不压缩，读完整个应答。是同步应答、里面有 `stale` 那个房间时，见过
/// 它的加入事件就把 `required_state` 里本人的成员事件换成加入前的邀请（见过才会造，见 [`report_as_invited`]）。
/// 返回是否改写了。`body` 是代理已经读下的请求体。
pub async fn forward_one(
    mut tcp: TcpStream,
    tuwunel: &str,
    head: &[u8],
    body: &[u8],
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
    let mut upstream = TcpStream::connect(tuwunel).await.unwrap_or_else(|error| {
        panic!(
            "failed to connect to Tuwunel at {tuwunel} for {}: {error}",
            head.lines().next().unwrap_or_default()
        )
    });
    upstream.write_all(request.as_bytes()).await.unwrap();
    upstream.write_all(body).await.unwrap();
    let mut response = Vec::new();
    let _ = upstream.read_to_end(&mut response).await;
    let is_sync = head
        .lines()
        .next()
        .is_some_and(|line| line.contains("/sync"));
    let rewritten = is_sync.then(|| rewrite(&response, stale)).flatten();
    // 改写要真的送达手机才算：同步重启的间隙里 SDK 可能已废弃这条连接（blip 后的重试、紧接着的下一次
    // 重启），改写落上去谁也看不见；这时保持 arm，让带同一个加入事件的下一个应答再试。
    let changed = rewritten.is_some();
    let delivered = tcp
        .write_all(rewritten.as_deref().unwrap_or(&response))
        .await
        .is_ok();
    let _ = tcp.shutdown().await;
    changed && delivered
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

/// `required_state` 里本人的成员事件换成加入前的邀请：时间线里有本人的加入事件（加入前是邀请）时换成那次邀请，
/// 已经见过加入事件、这条应答里却没有了时按房间成员现场造一条。
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
        // 还没见过加入事件的应答是加入前就在飞的，改写了也只是提前记账：原样放过。
        if !stale.seen_join.load(Ordering::SeqCst) {
            return false;
        }
        // 加入事件已经被别的应答吃掉：房间还在列表里就照常改错标，邀请现场造。
        // 成员事件只影响行的成员状态，不碰时间线。
        let inviter = room["required_state"]
            .as_array()
            .into_iter()
            .flatten()
            .chain(room["timeline"].as_array().into_iter().flatten())
            .filter(|event| event["type"] == "m.room.member")
            .find_map(|event| {
                let sender = event["sender"].as_str()?;
                (sender != stale.user_id).then_some(sender)
            })
            .unwrap_or(stale.user_id.as_str());
        let invite = serde_json::json!({
            "type": "m.room.member",
            "state_key": stale.user_id,
            "event_id": "$stale-invite",
            "sender": inviter,
            "content": {"membership": "invite"},
        });
        let required = room
            .as_object_mut()
            .unwrap()
            .entry("required_state")
            .or_insert_with(|| Value::Array(Vec::new()));
        let required = required.as_array_mut().unwrap();
        required.retain(|event| !is_own_member(event));
        required.push(invite);
        return true;
    };
    // 先置位再改写：这条连接可能被 SDK 丢掉，改写没送达也要让后面的应答继续改。
    stale.seen_join.store(true, Ordering::SeqCst);
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
