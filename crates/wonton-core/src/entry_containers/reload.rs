//! 入口 HAProxy 的软重载：经它主进程的管理端口（`-S`）发 `reload`，不带 Docker 的 kill。
//!
//! 经 Docker 发的信号都当停止处理，`unless-stopped` 的容器在守护进程重启后不再拉起（ADR-0116）。

use std::time::Duration;

use super::{ENTRY_COMPONENT, EntryContainers, EntryError};
use crate::plugin::names::{LABEL_COMPONENT, LABEL_MANAGED_BY};

/// 主进程处理上一次启动/重载时的固定回应，跟着 `Success=0` 一起到。
const IN_PROGRESS_TEXT: &str = "Another reload is still in progress";
const IN_PROGRESS: &[u8] = IN_PROGRESS_TEXT.as_bytes();

/// 主进程还没开始回应：隔一小段重连再发。
const RETRY_DELAY: Duration = Duration::from_millis(250);
/// 读回应的切片：一条接了不回的连接最多占这么久，收过数据就继续读。
const READ_SLICE: Duration = Duration::from_secs(1);
/// 从第一次尝试算起的截止；主进程处理完初始装载就会接。
const PENDING: Duration = Duration::from_secs(10);

impl EntryContainers {
    /// 让入口的 HAProxy 主进程软重载，见 [`reload`]。
    pub async fn reload(&self, addr: &str) -> Result<(), EntryError> {
        self.reload_checked(addr).await
    }

    pub(super) async fn reload_checked(&self, addr: &str) -> Result<(), EntryError> {
        let found = self
            .engine
            .labeled(LABEL_MANAGED_BY, "core", |labels| {
                self.instance.owns(labels)
                    && labels.get(LABEL_COMPONENT).map(String::as_str) == Some(ENTRY_COMPONENT)
            })
            .await?;
        match found.containers.as_slice() {
            [_] => reload_haproxy(addr).await,
            other => Err(EntryError::NotOneEntry(other.len())),
        }
    }
}

/// 连入口的主进程管理端口发 `reload`，等它回 `Success=1`。
///
/// 容器刚起来时管理端口先 accept、主进程还在做初始装载：第一次 `reload` 会立刻连上并回
/// `Success=0 / Another reload is still in progress`（3.4-alpine 实测），这时要隔一小段重发，
/// 否则刚建的入口会被当成“配置被拒”回滚。连上后一直不回应才隔一小段重连；已经收到字节就一直
/// 读到关连接或 10 秒截止，启动日志不会因为连接不关而丢掉。带着启动日志的 `Success=0` 是新配置
/// 真被拒，原样带回错误里，主人看到的不是一句“热加载失败”。
pub async fn reload_haproxy(addr: &str) -> Result<(), EntryError> {
    let deadline = tokio::time::Instant::now() + PENDING;
    loop {
        let reason = match attempt(addr, deadline).await {
            Attempt::Done => return Ok(()),
            Attempt::Rejected(logs) => return Err(EntryError::Reload(logs)),
            Attempt::Retry(reason) => reason,
        };
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return Err(EntryError::Reload(format!(
                "{addr}：10 秒内没有完成重载（{reason}）"
            )));
        }
        tokio::time::sleep(remaining.min(RETRY_DELAY)).await;
    }
}

enum Attempt {
    Done,
    Retry(String),
    Rejected(String),
}

async fn attempt(addr: &str, deadline: tokio::time::Instant) -> Attempt {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let mut stream =
        match tokio::time::timeout_at(deadline, tokio::net::TcpStream::connect(addr)).await {
            Err(_) => return Attempt::Retry("连管理端口超时".to_owned()),
            Ok(Err(error)) => return Attempt::Retry(format!("连不上：{error}")),
            Ok(Ok(stream)) => stream,
        };
    if let Err(error) = stream.write_all(b"reload\n").await {
        return Attempt::Retry(format!("发不出 reload：{error}"));
    }
    // 回应可能分几段到：切短读，静默才重连；收到数据就一直读到关连接或截止，到点也先分类。
    let mut reply = Vec::new();
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            break;
        }
        let mut buffer = [0u8; 4096];
        match tokio::time::timeout(remaining.min(READ_SLICE), stream.read(&mut buffer)).await {
            Err(_) if reply.is_empty() => {
                return Attempt::Retry("主进程还没有回应".to_owned());
            }
            Err(_) => continue,
            Ok(Ok(0)) => break,
            Ok(Err(error)) => {
                return Attempt::Retry(format!("主进程还没说完就断了：{error}{}", partial(&reply)));
            }
            Ok(Ok(n)) => {
                reply.extend_from_slice(&buffer[..n]);
                if reply.windows(9).any(|window| window == b"Success=1") {
                    return Attempt::Done;
                }
                // 主进程忙时可能先回这句、连接还挂着：不用等它关。
                if reply
                    .windows(IN_PROGRESS.len())
                    .any(|window| window == IN_PROGRESS)
                {
                    return Attempt::Retry(IN_PROGRESS_TEXT.to_owned());
                }
            }
        }
    }
    let reply = String::from_utf8_lossy(&reply);
    if reply.contains("Success=1") {
        Attempt::Done
    } else if let Some((_, logs)) = reply.split_once("Success=0\n--\n") {
        if logs.trim().starts_with(IN_PROGRESS_TEXT) {
            Attempt::Retry(logs.trim().to_owned())
        } else {
            Attempt::Rejected(logs.trim().to_owned())
        }
    } else if reply.trim().is_empty() {
        Attempt::Retry("主进程还没开始回应".to_owned())
    } else {
        Attempt::Retry(format!("回应不完整：{}", reply.trim()))
    }
}

fn partial(reply: &[u8]) -> String {
    let text = String::from_utf8_lossy(reply);
    let text = text.trim();
    if text.is_empty() {
        String::new()
    } else {
        format!("（已收到：{text}）")
    }
}
