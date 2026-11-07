//! Matrix 路径的假服务端：按测试的开关拒绝、挂起或断开特定请求，其余请求连同应答原样转发给
//! Tuwunel。`Core` 的字段就是这些开关，见 `support`。

use std::sync::{Arc, atomic::Ordering};

use tokio::io::AsyncWriteExt;
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
    if text.starts_with("get /_matrix/client/v3/room_keys/version") {
        let mut hold = core.hold_backup_read.subscribe();
        if hold.wait_for(|hold| !hold).await.is_err() {
            return;
        }
    }
    let verification_status = core.verification_failure.load(Ordering::SeqCst);
    if verification_status != 0 && text.starts_with("put /_matrix/client/v3/sendtodevice/") {
        if verification_status == 502 {
            // 传输故障：不回任何应答直接断开。SDK 默认对 5xx 应答会退避重试最长 15 分钟，
            // 连接错误没有重试上限才立即失败，和隧道断开时的行为一致。
            return;
        }
        let body = r#"{"errcode":"M_UNKNOWN","error":"Verification send rejected"}"#;
        let response = format!(
            "HTTP/1.1 {verification_status} Verification failed\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
            body.len()
        );
        tcp.write_all(response.as_bytes()).await.ok();
        tcp.shutdown().await.ok();
        return;
    }
    if text.starts_with("post /_matrix/client/v3/room_keys/version ")
        && core.reject_backup_creation.load(Ordering::SeqCst)
    {
        let body = r#"{"errcode":"M_FORBIDDEN","error":"Backup creation rejected"}"#;
        let response = format!(
            "HTTP/1.1 403 Forbidden\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\
                 connection: close\r\n\r\n{body}",
            body.len()
        );
        tcp.write_all(response.as_bytes()).await.unwrap();
        tcp.shutdown().await.unwrap();
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
    let mut stale = core.stale_join.subscribe();
    let armed = stale.borrow().clone();
    if let Some(armed) = armed {
        if forward_one(tcp, &core.tuwunel, &head, &armed).await {
            core.stale_join.send_replace(None);
            core.join_reported_as_invited.store(true, Ordering::SeqCst);
        }
        return;
    }
    if expire_room_list(&mut tcp, &core, text, &mut head).await {
        return;
    }
    let Some(mut upstream) = forward_to(&core, &head).await else {
        return;
    };
    tokio::select! {
        _ = tokio::io::copy_bidirectional(&mut tcp, &mut upstream) => {}
        _ = down.wait_for(|down| *down) => {}
        _ = stale.wait_for(Option::is_some) => {}
    }
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

/// 要求过期时，房间列表的下一个同步请求回 `M_UNKNOWN_POS`。连接号在请求体里，所以先读完请求体，别的同步请求连同
/// 请求体照常转发。回真时这个请求已经答完（或者读不下去了）。
async fn expire_room_list(
    tcp: &mut TcpStream,
    core: &Core,
    text: &str,
    head: &mut Vec<u8>,
) -> bool {
    if !text.starts_with("post /_matrix/client/unstable/org.matrix.simplified_msc3575/sync")
        || !core.expire_room_list.load(Ordering::SeqCst)
    {
        return false;
    }
    let Some(body) = read_body(tcp, text).await else {
        return true;
    };
    if String::from_utf8_lossy(&body).contains(r#""conn_id":"room-list""#)
        && core.expire_room_list.swap(false, Ordering::SeqCst)
    {
        let body = r#"{"errcode":"M_UNKNOWN_POS","error":"Unknown position"}"#;
        let response = format!(
            "HTTP/1.1 400 Bad Request\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\
             connection: close\r\n\r\n{body}",
            body.len()
        );
        tcp.write_all(response.as_bytes()).await.ok();
        tcp.shutdown().await.ok();
        return true;
    }
    head.extend_from_slice(&body);
    false
}
