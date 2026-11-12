//! 把刚加入的房间在同步应答里错标成“受邀”：时间线里有加入事件，`required_state` 里本人却还是邀请（Tuwunel
//! v2.0.0 之前 #685 的样子）。测试用它让列表行在受邀和已加入之间抖动。

use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

/// 要改写的房间和本人。
#[derive(Clone)]
pub struct StaleJoin {
    pub room_id: String,
    pub user_id: String,
}

/// 只转一个请求：让 Tuwunel 回完就断开、不压缩，读完整个应答。是同步应答、里面有 `stale` 那个房间的加入事件，
/// 就把 `required_state` 里本人的成员事件换成加入前的邀请。返回是否改写了。`body` 是代理已经读下的请求体。
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
