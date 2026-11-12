//! 模拟的核心服务端：`POST /_wonton/matrix/login-token` 签发主人的登录令牌，`PUT`、`DELETE
//! `/_wonton/push/registration` 记下推送登记（响应同核心的推送登记接口），`matrix.<基础域名>` 上的请求原样转给
//! Tuwunel。本地代理每个请求一条连接，读完请求头就能分流。

use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinHandle;

use super::{BASE_DOMAIN, Core, matrix, read_body, respond};

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
    answer_core_route(&core, &text, &mut tcp).await;
    super::request_done(request);
}

/// 假核心自己的接口：读完请求体，交给 Bot 的应答或 `respond`，没有就回 404。
async fn answer_core_route(core: &Core, text: &str, tcp: &mut TcpStream) {
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
        None => respond(core, text, &body),
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
