//! 模拟的核心服务端：`POST /_wonton/matrix/login-token` 签发主人的登录令牌，`POST /_wonton/api/pairing-codes`
//! 发“添加设备”的码，`PUT`、`DELETE` `/_wonton/push/registration` 记下推送登记（响应同核心的推送登记接口），
//! `GET /_wonton/api/confirmations` 给“等你确认”列表和单张卡片（ADR-0016），`matrix.<基础域名>` 上的请求原样转给
//! Tuwunel。本地代理每个请求一条连接，读完请求头就能分流。

use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinHandle;
use wonton_matrix::testkit::PushGateway;

use super::{BASE_DOMAIN, Core, OWNER, matrix, read_body};

pub(super) async fn fake_core(core: Arc<Core>) -> (SocketAddr, JoinHandle<()>) {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let addr = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        loop {
            let (tcp, _) = listener.accept().await.unwrap();
            tokio::spawn(serve(tcp, core.clone()));
        }
    });
    (addr, task)
}

async fn serve(mut tcp: TcpStream, core: Arc<Core>) {
    let mut head = Vec::new();
    while !head.ends_with(b"\r\n\r\n") {
        let mut byte = [0u8; 1];
        if tcp.read(&mut byte).await.unwrap_or(0) == 0 {
            return;
        }
        head.push(byte[0]);
    }
    let text = String::from_utf8_lossy(&head).to_ascii_lowercase();
    if text.contains(&format!("\r\nhost: matrix.{BASE_DOMAIN}")) {
        core.bot_api.observe_matrix(&text);
        let request = super::request_arrived(&text);
        matrix::serve_matrix(tcp, core, head, &text).await;
        super::request_done(request);
        return;
    }
    let request = super::request_arrived(&text);
    let raw = String::from_utf8_lossy(&head).into_owned();
    answer_core_route(&core, &text, &raw, &mut tcp).await;
    super::request_done(request);
}

/// 假核心自己的接口：tus 的路由自己写应答（要带 tus 的头），读完请求体，交给 Bot 的应答或 `respond`，没有就回 404。
/// `raw` 是没转小写的请求头：`Upload-Metadata` 里的 base64 分大小写。
async fn answer_core_route(core: &Arc<Core>, text: &str, raw: &str, tcp: &mut TcpStream) {
    if super::files::serve(core, text, raw, tcp).await {
        return;
    }
    if super::link_preview::serve(core, text, tcp).await {
        return;
    }
    let Some(body) = read_body(tcp, text).await else {
        return;
    };
    if text.starts_with("post /_wonton/confirmations/")
        || text.starts_with("delete /_wonton/confirmations/")
    {
        let mut released = core.hold_answers.subscribe();
        released.wait_for(|hold| !hold).await.ok();
    }
    let reply = match core.bot_api.respond(text, &body).await {
        Some(reply) => Some(reply),
        None => respond(core, text, &body).await,
    };
    hold_push_registration_response(core, text).await;
    let response = match reply {
        Some((status, body)) => format!(
            "HTTP/1.1 {status} Whatever\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\
             connection: close\r\n\r\n{body}",
            body.len()
        ),
        None => {
            "HTTP/1.1 404 Not Found\r\ncontent-length: 0\r\nconnection: close\r\n\r\n".to_owned()
        }
    };
    tcp.write_all(response.as_bytes()).await.ok();
    tcp.shutdown().await.ok();
}

/// 回归测试：登记请求已记下（见 `respond`），响应先不回，后台登记就停在“PUT 已收到、还没登记完”。
async fn hold_push_registration_response(core: &Core, text: &str) {
    if (text.starts_with("put /_wonton/push/registration ")
        || text.starts_with("delete /_wonton/push/registration "))
        && *core.hold_push_registrations.borrow()
    {
        let mut released = core.hold_push_registrations.subscribe();
        released.wait_for(|hold| !hold).await.ok();
    }
}

/// 核心自己的接口；不认识的请求回 `None`（404）。
async fn respond(core: &Core, head: &str, body: &[u8]) -> Option<(u16, String)> {
    let push = |method: &str, body: Value| {
        core.push_requests
            .lock()
            .unwrap()
            .push((method.to_owned(), body));
        core.push_registrations_seen
            .send_modify(|count| *count += 1);
    };
    if let Some(rest) = head.strip_prefix("post /_wonton/confirmations/") {
        let id = rest
            .split_whitespace()
            .next()
            .unwrap_or_default()
            .to_owned();
        let answer: Value = serde_json::from_slice(body).unwrap_or(Value::Null);
        let state = match answer["answer"].as_str() {
            Some("approve") => "running",
            _ => "rejected",
        };
        core.answers.lock().unwrap().push((id, answer));
        let (status, reply) = core
            .answer_replies
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| (200, json!({"state": {"type": state}})));
        return Some((status, reply.to_string()));
    }
    if let Some(rest) = head.strip_prefix("delete /_wonton/confirmations/") {
        let id = rest.split('/').next().unwrap_or_default().to_owned();
        core.window_stops.lock().unwrap().push(id);
        let reply = core.answer_replies.lock().unwrap().pop_front();
        return Some(reply.map_or((204, String::new()), |(status, reply)| {
            (status, reply.to_string())
        }));
    }
    if let Some(reply) = super::friends::respond(core, head, body) {
        return Some(reply);
    }
    if head.starts_with("get /_wonton/api/confirmations") {
        return confirmations(core, head).await;
    }
    if head.starts_with("get /_wonton/api/plugins ") {
        core.installed_requests.fetch_add(1, Ordering::SeqCst);
        let installed = core.installed.lock().unwrap().clone()?;
        return Some((200, installed.to_string()));
    }
    let response = if head.starts_with("post /_wonton/matrix/login-token ") {
        if core
            .failing_login_tokens
            .try_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1))
            .is_ok()
        {
            return Some((502, String::new()));
        }
        core.login_tokens.fetch_add(1, Ordering::SeqCst);
        json!({
            "homeserver_url": format!("http://matrix.{BASE_DOMAIN}"),
            "server_name": BASE_DOMAIN,
            "user_id": OWNER,
            "login_type": "org.matrix.login.jwt",
            "token": core.issuer.login_token("owner"),
            "expires_in": 120,
        })
    } else if head.starts_with("put /_wonton/push/registration ") {
        push("PUT", serde_json::from_slice(body).unwrap_or(Value::Null));
        json!({"pushkey": core.pushkey, "gateway_url": PushGateway::URL})
    } else if head.starts_with("post /_wonton/api/pairing-codes ") {
        let code = core.issued_pairing_code.lock().unwrap().clone()?;
        json!({"code": code, "expires_in": 600})
    } else if head.starts_with("get /_wonton/api/onboarding ") {
        let value = core.onboarding.lock().unwrap().clone()?;
        return Some((200, value.to_string()));
    } else if head.starts_with("delete /_wonton/push/registration ") {
        push("DELETE", Value::Null);
        json!({"removed": true})
    } else {
        return None;
    };
    Some((200, response.to_string()))
}

/// 假核心的确认接口（ADR-0016）：单张 `GET /_wonton/api/confirmations/{id}` 和列表
/// `GET /_wonton/api/confirmations?after=<revision>`。带 `after` 且没有更新时挂住等 revision 变化，
/// 和真核心一样，客户端的长轮询才不会空转。
async fn confirmations(core: &Core, head: &str) -> Option<(u16, String)> {
    let rest = head.strip_prefix("get /_wonton/api/confirmations")?;
    if let Some(rest) = rest.strip_prefix('/') {
        let id = rest.split_whitespace().next().unwrap_or_default();
        core.single_confirmation_requests
            .fetch_add(1, Ordering::SeqCst);
        if core.single_confirmation_missing.load(Ordering::SeqCst) {
            return Some((404, String::new()));
        }
        if core
            .failing_single_confirmation
            .try_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1))
            .is_ok()
        {
            return Some((503, String::new()));
        }
        let list = core.confirmations.lock().unwrap().clone();
        let wanted = serde_json::json!(id);
        return Some(
            match list
                .pending
                .iter()
                .chain(&list.finished)
                .find(|card| card["id"] == wanted)
            {
                Some(card) => (200, card.to_string()),
                None => (404, String::new()),
            },
        );
    }
    core.confirmations_requests.fetch_add(1, Ordering::SeqCst);
    if core
        .failing_confirmations
        .try_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1))
        .is_ok()
    {
        return Some((502, String::new()));
    }
    let list = core.confirmations.lock().unwrap().clone();
    let after = head
        .split("?after=")
        .nth(1)
        .and_then(|rest| rest.split_whitespace().next())
        .and_then(|after| after.parse::<u64>().ok());
    if let Some(after) = after
        && after >= list.revision
    {
        let mut revision = core.confirmations_revision.subscribe();
        drop(list);
        // 和真核心一样挂到有更新为止，测试里限两秒：没有更新就回当前列表，客户端再去取，
        // 不让测试等满整个客户端超时。
        let _ = tokio::time::timeout(
            Duration::from_secs(2),
            revision.wait_for(|revision| *revision > after),
        )
        .await;
    }
    let list = core.confirmations.lock().unwrap().clone();
    Some((
        200,
        json!({
            "confirmations": list.pending,
            "finished": list.finished,
            "contact_requests": list.contact_requests,
            "revision": list.revision,
        })
        .to_string(),
    ))
}
