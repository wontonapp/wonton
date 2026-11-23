//! 入口 HAProxy 的软重载：经它主进程的管理端口（`-S`）发 `reload`，不带 Docker 的 kill。
//!
//! 经 Docker 发的信号都当停止处理，`unless-stopped` 的容器在守护进程重启后不再拉起（ADR-0115）。

use std::time::Duration;

use super::{ENTRY_COMPONENT, EntryContainers, EntryError};
use crate::plugin::names::{LABEL_COMPONENT, LABEL_MANAGED_BY};

impl EntryContainers {
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
/// 主进程可能还没起来（刚对账过、容器在重启）：连不上就重试到 5 秒。`Success=0` 后面是新的启动日志，
/// 原样带回错误里，主人看到的不是一句"热加载失败"。
pub async fn reload_haproxy(addr: &str) -> Result<(), EntryError> {
    // 主进程刚起步、上一次重载还没完时，连接会被关掉或回“还在重载”：都还来得及重试。
    let mut last = String::new();
    for _ in 0..40 {
        match try_reload(addr).await {
            Ok(()) => return Ok(()),
            Err(Attempt::Retry(cause)) => last = cause,
            Err(Attempt::Rejected(logs)) => return Err(EntryError::Reload(logs)),
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    Err(EntryError::Reload(format!(
        "连上 {addr} 也一直没接受重载：{last}"
    )))
}

enum Attempt {
    /// 连不上、还没说起、或主进程明确说上一次还在重载：过一会儿再来。
    Retry(String),
    /// 新配置起不来，带着启动日志。
    Rejected(String),
}

/// 连管理端口发一次 `reload`，读到主进程的结论。
async fn try_reload(addr: &str) -> Result<(), Attempt> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpStream;

    let mut stream = TcpStream::connect(addr)
        .await
        .map_err(|error| Attempt::Retry(format!("连不上：{error}")))?;
    stream
        .write_all(b"reload\n")
        .await
        .map_err(|error| Attempt::Retry(format!("发不出 reload：{error}")))?;
    // 回应可能分几段到：读到关连接或 10 秒为止，失败时的启动日志也这样收全。
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    let mut reply = Vec::new();
    loop {
        let mut buffer = [0u8; 4096];
        match tokio::time::timeout_at(deadline, stream.read(&mut buffer)).await {
            Err(_) | Ok(Ok(0)) | Ok(Err(_)) => break,
            Ok(Ok(n)) => {
                reply.extend_from_slice(&buffer[..n]);
                if reply.windows(9).any(|w| w == b"Success=1") {
                    return Ok(());
                }
                // 拒绝的回应以“Success=0”开头，后面是日志：看到就不再等。
                if reply.starts_with(b"Success=0\n") && reply.windows(4).any(|w| w == b"\n--\n") {
                    break;
                }
            }
        }
    }
    let reply = String::from_utf8_lossy(&reply);
    if reply.contains("Success=1") {
        Ok(())
    } else if let Some((_, logs)) = reply.split_once("Success=0\n--\n") {
        if logs.contains("still in progress") {
            Err(Attempt::Retry(format!("主进程还在重载：{}", logs.trim())))
        } else {
            Err(Attempt::Rejected(logs.trim().to_owned()))
        }
    } else if reply.trim().is_empty() {
        Err(Attempt::Retry("主进程还没说起".to_owned()))
    } else {
        Err(Attempt::Rejected(reply.trim().to_owned()))
    }
}
