//! 登录的替身：`authenticate` 的 URL 登录流程和终端登录的伪进程。

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use serde_json::{Value, json};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::{UnixStream, unix::OwnedWriteHalf},
    sync::Mutex,
};

use super::{Scenario, bounded, reply, request, rpc_error};

pub(super) async fn handle_authenticate(
    writer: &Arc<Mutex<OwnedWriteHalf>>,
    id: &Value,
    lines: &mut tokio::io::Lines<BufReader<tokio::net::unix::OwnedReadHalf>>,
    scenario: &Scenario,
    logged_in: &Arc<AtomicBool>,
) {
    if !matches!(scenario, Scenario::LoginUrl) {
        rpc_error(writer, id, -32601, "Method not found").await;
        return;
    }
    let request_id = json!(9_000);
    request(
        writer,
        &request_id,
        "elicitation/create",
        json!({
            "mode": "url",
            "elicitationId": "login-1",
            "url": "https://auth.example/login?code=abc",
            "message": "请打开链接完成登录"
        }),
    )
    .await;
    let action = loop {
        let Ok(Some(line)) = bounded(lines.next_line(), "等待 elicitation 应答")
            .await
            .map_err(|_| ())
        else {
            return;
        };
        let Ok(message) = serde_json::from_str::<Value>(&line) else {
            return;
        };
        if message["id"] == request_id {
            break message["result"]["action"]
                .as_str()
                .unwrap_or("cancel")
                .to_owned();
        }
    };
    if action == "accept" {
        logged_in.store(true, Ordering::Relaxed);
        reply(writer, id, json!({})).await;
    } else {
        rpc_error(writer, id, -32000, "登录被拒绝").await;
    }
}

pub(super) async fn serve_terminal_login(stream: UnixStream, logged_in: Arc<AtomicBool>) {
    let (reader, mut writer) = stream.into_split();
    let mut lines = BufReader::new(reader).lines();
    let Some(request) = lines.next_line().await.unwrap() else {
        return;
    };
    let request: Value = serde_json::from_str(&request).unwrap();
    assert_eq!(request["methodId"], "term");
    assert_eq!(request["args"][0], "--login");
    // 链接打在 stderr，验证桥也把它送进聊天。
    send_frame(&mut writer, json!({"err": "https://auth.example/device"})).await;
    send_frame(&mut writer, json!({"out": "输入授权码："})).await;
    for line in [] as [&str; 0] {
        send_frame(&mut writer, json!({"out": line})).await;
    }
    let Some(code) = lines.next_line().await.unwrap() else {
        return;
    };
    assert_eq!(code, "code-42");
    logged_in.store(true, Ordering::Relaxed);
    send_frame(&mut writer, json!({"exit": 0})).await;
}

async fn send_frame(writer: &mut tokio::net::unix::OwnedWriteHalf, frame: Value) {
    writer
        .write_all(frame.to_string().as_bytes())
        .await
        .unwrap();
    writer.write_all(b"\n").await.unwrap();
    writer.flush().await.unwrap();
}
