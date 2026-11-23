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
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpStream;

    let mut last = None;
    let mut stream = None;
    for _ in 0..20 {
        match TcpStream::connect(addr).await {
            Ok(found) => {
                stream = Some(found);
                break;
            }
            Err(error) => last = Some(error),
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    let Some(mut stream) = stream else {
        return Err(EntryError::Reload(format!(
            "连不上 {addr}：{}",
            last.map_or_else(|| "没有尝试".to_owned(), |error| error.to_string())
        )));
    };
    stream
        .write_all(b"reload\n")
        .await
        .map_err(|error| EntryError::Reload(format!("发不出 reload：{error}")))?;
    // 回应可能分几段到：读到关连接或 10 秒为止，失败时的启动日志也这样收全。
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    let mut reply = Vec::new();
    loop {
        let mut buffer = [0u8; 4096];
        match tokio::time::timeout_at(deadline, stream.read(&mut buffer)).await {
            Err(_) if reply.is_empty() => {
                return Err(EntryError::Reload("主进程 10 秒没有回应".to_owned()));
            }
            Err(_) | Ok(Ok(0)) | Ok(Err(_)) => break,
            Ok(Ok(n)) => {
                reply.extend_from_slice(&buffer[..n]);
                if reply.windows(9).any(|w| w == b"Success=1") {
                    break;
                }
            }
        }
    }
    let reply = String::from_utf8_lossy(&reply);
    if reply.contains("Success=1") {
        Ok(())
    } else if let Some((_, logs)) = reply.split_once("Success=0\n--\n") {
        Err(EntryError::Reload(logs.trim().to_owned()))
    } else {
        Err(EntryError::Reload(reply.trim().to_owned()))
    }
}
