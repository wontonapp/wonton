//! 终端类登录：桥连 Agent 入口脚本监听的登录控制 socket。
//!
//! socket 协议（与 Agent 镜像入口约定，`deploy/dev` 的样例入口是实现）：
//! 桥先发一行 JSON `{"methodId","args","env"}`；入口启动同一配置的 Agent 程序，
//! 之后桥发普通文本行写入它的 stdin；入口回 JSON 行 `{"out"}`、`{"err"}`（各一行）
//! 和结束帧 `{"exit": 数字}`。`exit` 为 0 才算成功（ACP 终端登录以退出码为准）。

use std::path::Path;

use serde_json::{Value, json};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::UnixStream,
    sync::mpsc,
};

use super::LoginEvent;
use crate::{
    acp::auth::AuthMethod,
    error::{Error, Result},
};

pub async fn run(
    socket: &Path,
    method: &AuthMethod,
    mut input: mpsc::Receiver<String>,
    events: mpsc::Sender<LoginEvent>,
) -> Result<()> {
    let login_socket = socket.with_extension("login.sock");
    let stream = UnixStream::connect(&login_socket).await.map_err(|error| {
        Error::AgentNotReady(format!(
            "连接登录控制 socket {} 失败：{error}",
            login_socket.display()
        ))
    })?;
    let (reader, mut writer) = stream.into_split();
    let request = json!({"methodId": method.id, "args": method.args, "env": method.env});
    write_line(&mut writer, &request.to_string()).await?;
    let mut frames = BufReader::new(reader).lines();
    let mut stderr_tail: Option<String> = None;
    let mut exit: Option<i64> = None;
    loop {
        tokio::select! {
            frame = frames.next_line() => match frame {
                Ok(Some(frame)) => {
                    let Ok(value) = serde_json::from_str::<Value>(&frame) else {
                        return Err(Error::Upstream("登录 socket 返回了无效帧".into()));
                    };
                    if let Some(code) = value["exit"].as_i64() {
                        exit = Some(code);
                        break;
                    }
                    if let Some(line) = value["out"].as_str().filter(|line| !line.is_empty()) {
                        let _ = events.send(LoginEvent::Output(line.to_owned())).await;
                    }
                    // 不少 CLI 把设备码链接打在 stderr，也逐行进聊天；尾部留一份给失败原因。
                    if let Some(line) = value["err"].as_str().filter(|line| !line.is_empty()) {
                        stderr_tail = Some(line.to_owned());
                        let _ = events.send(LoginEvent::Output(line.to_owned())).await;
                    }
                }
                _ => break,
            },
            Some(code) = input.recv() => write_line(&mut writer, &code).await?,
        }
    }
    let Some(code) = exit else {
        return Err(Error::Upstream("Agent 登录进程提前退出".into()));
    };
    if code != 0 {
        if let Some(line) = stderr_tail {
            let mut line = line;
            line.truncate(2000);
            return Err(Error::Upstream(format!("Agent 登录失败：{line}")));
        }
        return Err(Error::Upstream(format!(
            "Agent 登录进程以退出码 {code} 失败"
        )));
    }
    Ok(())
}

async fn write_line(writer: &mut tokio::net::unix::OwnedWriteHalf, line: &str) -> Result<()> {
    writer
        .write_all(line.as_bytes())
        .await
        .map_err(|error| Error::AgentNotReady(format!("登录 socket 已断开：{error}")))?;
    writer
        .write_all(b"\n")
        .await
        .map_err(|error| Error::AgentNotReady(format!("登录 socket 已断开：{error}")))?;
    writer
        .flush()
        .await
        .map_err(|error| Error::AgentNotReady(format!("登录 socket 已断开：{error}")))
}
