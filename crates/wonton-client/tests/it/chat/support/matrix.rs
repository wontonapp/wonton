//! Matrix 路径的假服务端：按测试的开关拒绝、挂起或断开特定请求，其余请求连同应答原样转发给
//! Tuwunel。`Core` 的字段就是这些开关，见 `support`。

use std::sync::{Arc, atomic::Ordering};
use std::time::Duration;

use serde_json::Value;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tracing::debug;

use super::super::stale_join::forward_one;
use super::{Core, read_body};

pub(super) async fn serve_matrix(
    mut tcp: TcpStream,
    core: Arc<Core>,
    mut head: Vec<u8>,
    text: &str,
) {
    if intercept_verification(&mut tcp, &core, text).await {
        return;
    }
    if intercept_backup_creation(&mut tcp, &core, text).await {
        return;
    }
    if text.starts_with("post /_matrix/media/")
        && text
            .lines()
            .next()
            .is_some_and(|line| line.contains("/upload"))
    {
        let mut paused = core.uploads_paused.subscribe();
        paused.wait_for(|paused| !paused).await.unwrap();
    }
    // `chat::key_upload_order` 回归测试：挂起期间手机的 keys/upload 一接上就断开（drop 语义），
    // 放行后才转发并计数；SDK 那头只记一条 outgoing 请求失败的日志，下一轮同步自动重试。
    if text.starts_with("post /_matrix/client/v3/keys/upload") {
        if core.hold_key_uploads.load(Ordering::SeqCst) {
            return;
        }
        core.key_uploads_forwarded.send_modify(|count| *count += 1);
    }
    if hold_key_request(&mut tcp, &core, text, &mut head)
        .await
        .is_none()
    {
        return;
    }
    if hold_done_for_key_request(&mut tcp, &core, text, &mut head)
        .await
        .is_none()
    {
        return;
    }
    let is_sync = text
        .lines()
        .next()
        .is_some_and(|line| line.contains("msc3575/sync"));
    // 此处只预读同步 body；上面的设备确认钩子仅在启用时预读对应的 secret request/done。
    // 其余请求保持流式转发，避免丢失加入请求的正文。
    let mut body = Vec::new();
    let mut request = None;
    if is_sync {
        let Some(read) = read_body(&mut tcp, text).await else {
            return;
        };
        body = read;
        request = record_sync_request(&core, text, &body).await;
    }
    let mut down = core.matrix_down.subscribe();
    if *down.borrow() {
        return;
    }
    let asks_for_members = text
        .lines()
        .next()
        .is_some_and(|line| line.contains("/members"));
    if asks_for_members && core.members_down.load(Ordering::SeqCst) {
        core.members_requests_dropped.fetch_add(1, Ordering::SeqCst);
        return;
    }
    if expire_room_list(&mut tcp, &core, request.as_ref()).await {
        return;
    }
    let mut stale = core.stale_join.subscribe();
    let armed = stale.borrow().clone();
    if let Some(armed) = armed.filter(|_| is_sync) {
        if forward_one(tcp, &core.tuwunel, &head, &body, &armed).await {
            core.join_reported_as_invited.fetch_add(1, Ordering::SeqCst);
        }
        return;
    }
    let Some(mut upstream) = forward_to(&core, &head).await else {
        return;
    };
    if text.starts_with(KEY_REQUEST) {
        core.key_requests_forwarded.send_modify(|count| *count += 1);
    }
    if is_sync {
        upstream.write_all(&body).await.unwrap();
    }
    tokio::select! {
        _ = tokio::io::copy_bidirectional(&mut tcp, &mut upstream) => {}
        _ = down.wait_for(|down| *down) => {}
        _ = stale.wait_for(Option::is_some), if is_sync => {}
    }
}

const KEY_REQUEST: &str = "put /_matrix/client/v3/sendtodevice/m.secret.request/";
const DONE: &str = "put /_matrix/client/v3/sendtodevice/m.key.verification.done/";

async fn intercept_backup_creation(tcp: &mut TcpStream, core: &Core, text: &str) -> bool {
    if !text.starts_with("post /_matrix/client/v3/room_keys/version ")
        || !core.reject_backup_creation.load(Ordering::SeqCst)
    {
        return false;
    }
    let body = r#"{"errcode":"M_FORBIDDEN","error":"Backup creation rejected"}"#;
    let response = format!(
        "HTTP/1.1 403 Forbidden\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\
         connection: close\r\n\r\n{body}",
        body.len()
    );
    tcp.write_all(response.as_bytes()).await.unwrap();
    tcp.shutdown().await.unwrap();
    true
}

/// Tuwunel 在重启时核心网关的应答：Matrix 的 JSON 错误 503。
async fn answer_unavailable(tcp: &mut TcpStream, core: &Core) -> bool {
    if !core.matrix_unavailable.load(Ordering::SeqCst) {
        return false;
    }
    let body = r#"{"errcode":"M_UNKNOWN","error":"Tuwunel 暂时无法访问"}"#;
    let response = format!(
        "HTTP/1.1 503 Service Unavailable\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    );
    tcp.write_all(response.as_bytes()).await.ok();
    tcp.shutdown().await.ok();
    true
}

/// 开了 `done_after_key_request` 时，要钥匙的请求和确认完成消息先读完请求体、整个转发。发给指定设备的完成消息
/// 由假核心先回 200、留在手里：新手机的同步不会因为这个请求没回而停下，照常收到对方的完成消息、发出要钥匙的请求；
/// 等要钥匙的请求转发出去，再过三秒让对方的同步先把它取走，才把完成消息补发给 Tuwunel。
/// 回 `None` 表示这个请求已经答完（或者读不下去了）。
async fn hold_done_for_key_request(
    tcp: &mut TcpStream,
    core: &Arc<Core>,
    text: &str,
    head: &mut Vec<u8>,
) -> Option<()> {
    if !text.starts_with(KEY_REQUEST) && !text.starts_with(DONE) {
        return Some(());
    }
    let Some(device_id) = core.done_after_key_request.lock().unwrap().clone() else {
        return Some(());
    };
    let body = read_body(tcp, text).await?;
    let to_device = String::from_utf8_lossy(&body).contains(&format!("\"{device_id}\""));
    head.extend_from_slice(&body);
    if !(text.starts_with(DONE) && to_device) {
        return Some(());
    }
    reply_ok(tcp).await;
    let (core, request) = (core.clone(), std::mem::take(head));
    tokio::spawn(async move {
        let mut forwarded = core.key_requests_forwarded.subscribe();
        // 等不到（新手机没要钥匙）也照常补发，让测试按原来的结果失败，不在这里挂住。
        tokio::time::timeout(
            Duration::from_secs(10),
            forwarded.wait_for(|count| *count > 0),
        )
        .await
        .ok();
        tokio::time::sleep(Duration::from_secs(3)).await;
        forward_held(&core, &request).await;
    });
    None
}

/// 开了 `hold_key_requests` 时，要秘密的请求由假核心先回 200、留在手里，关掉后再补发给 Tuwunel。
/// 回 `None` 表示这个请求已经答完（或者读不下去了）。
async fn hold_key_request(
    tcp: &mut TcpStream,
    core: &Arc<Core>,
    text: &str,
    head: &mut Vec<u8>,
) -> Option<()> {
    if !text.starts_with(KEY_REQUEST) || !*core.hold_key_requests.borrow() {
        return Some(());
    }
    let body = read_body(tcp, text).await?;
    head.extend_from_slice(&body);
    reply_ok(tcp).await;
    let (core, request) = (core.clone(), std::mem::take(head));
    tokio::spawn(async move {
        let mut hold = core.hold_key_requests.subscribe();
        hold.wait_for(|hold| !hold).await.ok();
        forward_held(&core, &request).await;
    });
    None
}

async fn reply_ok(tcp: &mut TcpStream) {
    let reply = "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: 2\r\nconnection: close\r\n\r\n{}";
    tcp.write_all(reply.as_bytes()).await.ok();
    tcp.shutdown().await.ok();
}

/// 补发留着的请求。连接是 keep-alive，读到应答头就够了：读完前断开，Tuwunel 可能放弃处理。
async fn forward_held(core: &Core, request: &[u8]) {
    let Some(mut upstream) = forward_to(core, request).await else {
        return;
    };
    let mut response = [0u8; 1024];
    tokio::time::timeout(Duration::from_secs(10), upstream.read(&mut response))
        .await
        .ok();
}

async fn intercept_verification(tcp: &mut TcpStream, core: &Core, text: &str) -> bool {
    if text.starts_with("get /_matrix/client/v3/room_keys/version") {
        let mut hold = core.hold_backup_read.subscribe();
        if hold.wait_for(|hold| !hold).await.is_err() {
            return true;
        }
    }
    if answer_unavailable(tcp, core).await {
        return true;
    }
    let status = core.verification_failure.load(Ordering::SeqCst);
    if status == 0 || !text.starts_with("put /_matrix/client/v3/sendtodevice/") {
        return false;
    }
    // SDK 对 5xx 退避重试；502 探针用直接断开连接模拟传输故障，沿用主干的失败分类。
    if status != 502 {
        let body = r#"{"errcode":"M_UNKNOWN","error":"Verification send rejected"}"#;
        let response = format!(
            "HTTP/1.1 {status} Verification failed\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
            body.len()
        );
        tcp.write_all(response.as_bytes()).await.ok();
        tcp.shutdown().await.ok();
    }
    true
}

/// 连上 Tuwunel 并把请求头转过去；连不上或写不进去就断开这条连接（客户端会重试）。
async fn forward_to(core: &Core, head: &[u8]) -> Option<TcpStream> {
    let mut upstream = match TcpStream::connect(core.tuwunel.as_str()).await {
        Ok(upstream) => upstream,
        Err(err) => {
            debug!("fake core cannot reach Tuwunel: {err}");
            return None;
        }
    };
    match upstream.write_all(head).await {
        Ok(()) => Some(upstream),
        Err(err) => {
            debug!("fake core cannot forward to Tuwunel: {err}");
            None
        }
    }
}

/// 下一条房间列表请求回 M_UNKNOWN_POS，过期注入先于房间状态改写。
async fn expire_room_list(tcp: &mut TcpStream, core: &Core, request: Option<&Value>) -> bool {
    let room_list = request
        .and_then(|request| request.get("conn_id"))
        .and_then(|conn| conn.as_str())
        == Some("room-list");
    if !room_list || !core.expire_room_list.swap(false, Ordering::SeqCst) {
        return false;
    }
    let body = r#"{"errcode":"M_UNKNOWN_POS","error":"Unknown position"}"#;
    let response = format!(
        "HTTP/1.1 400 Bad Request\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\
         connection: close\r\n\r\n{body}",
        body.len()
    );
    tcp.write_all(response.as_bytes()).await.ok();
    tcp.shutdown().await.ok();
    true
}

/// 记录请求体和查询里的 pos，区分重启的首条请求与长轮询。
async fn record_sync_request(core: &Core, text: &str, body: &[u8]) -> Option<Value> {
    let mut request: Value = serde_json::from_slice(body).ok()?;
    let pos = text
        .lines()
        .next()
        .and_then(|line| line.split("pos=").nth(1))
        .map(|rest| rest.split(['&', ' ']).next().unwrap_or("").to_owned())
        .filter(|pos| !pos.is_empty());
    let sequence = text
        .lines()
        .find_map(|line| {
            line.strip_prefix("x-wonton-test-sequence:")
                .and_then(|value| value.trim().parse::<u64>().ok())
        })
        .expect("sync request must have a local proxy sequence");
    request["test_sequence"] = sequence.into();
    request["pos"] = pos.map(Value::String).unwrap_or(Value::Null);
    // Joined 行可先于加入流程的临时订阅清理出现，不能把那条带订阅请求当成旧空长轮询。
    if request["conn_id"] == "room-list"
        && request.get("room_subscriptions").is_none()
        && core.hold_sync_recording.swap(false, Ordering::SeqCst)
    {
        core.sync_recording_held.send_replace(Some(request.clone()));
        let mut released = core.sync_recording_released.subscribe();
        released.wait_for(|released| *released).await.ok()?;
    }
    core.sync_requests.lock().unwrap().push(request.clone());
    Some(request)
}
