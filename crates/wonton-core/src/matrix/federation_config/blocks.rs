//! 整台服务器的封禁独立于个人邀请过滤，变动与配对范围一起热加载。

use std::collections::BTreeSet;
use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use anyhow::Context;
use tokio::sync::watch;

pub struct FederationBlocks {
    path: PathBuf,
    names: Mutex<BTreeSet<String>>,
    changed: watch::Sender<BTreeSet<String>>,
}

impl FederationBlocks {
    pub fn open(path: impl Into<PathBuf>) -> anyhow::Result<Self> {
        let path = path.into();
        let names = read(&path)?;
        let (changed, _) = watch::channel(names.clone());
        Ok(Self {
            path,
            names: Mutex::new(names),
            changed,
        })
    }

    pub fn names(&self) -> BTreeSet<String> {
        self.names.lock().unwrap().clone()
    }

    pub fn subscribe(&self) -> watch::Receiver<BTreeSet<String>> {
        self.changed.subscribe()
    }

    pub fn reload(&self) -> anyhow::Result<()> {
        let mut names = self.names.lock().unwrap();
        let _lock = self.lock(false)?;
        let updated = read(&self.path)?;
        if updated != *names {
            *names = updated.clone();
            self.changed.send_replace(updated);
        }
        Ok(())
    }

    /// 命令行的 `federation-block` 和常驻的核心改同一个文件：拿独占锁后重读再合并，
    /// 否则一边读到的旧内容会把另一边刚写进去的服务器盖掉。锁只保护这次读改写，不跨进程共享内存。
    pub fn set(&self, server: &str, blocked: bool) -> anyhow::Result<()> {
        validate_name(server)?;
        let mut names = self.names.lock().unwrap();
        let _lock = self.lock(true)?;
        let persisted = read(&self.path)?;
        let mut updated = persisted.clone();
        if blocked {
            updated.insert(server.to_owned());
        } else {
            updated.remove(server);
        }
        // 和内存相同也要看盘：外部进程可能刚改过，而内存还没刷到；那次改动不能因为早退丢掉。
        if updated != persisted {
            crate::install::write_private(&self.path, &serde_json::to_string(&updated)?)?;
        }
        if updated != *names {
            *names = updated.clone();
            self.changed.send_replace(updated);
        }
        Ok(())
    }

    /// 锁文件跟着数据文件放，单独一个 `.lock`：数据文件本身是原地重写加改名的，不能当锁用。
    /// 锁在 `File` 关掉时释放。
    fn lock(&self, exclusive: bool) -> anyhow::Result<File> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let path = self.path.with_extension("lock");
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .with_context(|| format!("打开锁文件 {}", path.display()))?;
        if exclusive {
            file.lock()
                .with_context(|| format!("锁定 {}", path.display()))?;
        } else {
            file.lock_shared()
                .with_context(|| format!("共享锁定 {}", path.display()))?;
        }
        Ok(file)
    }
}

/// 服务器名只能是小写 DNS 名字，可以带非默认端口（`example.org:8448`）。大小写归一由调用方做。
fn validate_name(name: &str) -> anyhow::Result<()> {
    let (host, port) = match name.rsplit_once(':') {
        Some((host, port)) => (host, Some(port)),
        None => (name, None),
    };
    let bad_port = port.is_some_and(|port| {
        port.is_empty()
            || port.len() > 5
            || !port.bytes().all(|c| c.is_ascii_digit())
            || port == "0"
    });
    anyhow::ensure!(
        !host.is_empty()
            && host.len() <= 253
            && !bad_port
            && host.split('.').all(|label| {
                !label.is_empty()
                    && label.len() <= 63
                    && !label.starts_with('-')
                    && !label.ends_with('-')
                    && label
                        .bytes()
                        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
            }),
        "服务器名必须是小写 DNS 名字，端口可省：{name}"
    );
    Ok(())
}

fn read(path: &Path) -> anyhow::Result<BTreeSet<String>> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(BTreeSet::new()),
        Err(e) => return Err(e).with_context(|| format!("读取 {}", path.display())),
    };
    let names: BTreeSet<String> = serde_json::from_str(&text)
        .with_context(|| format!("黑名单 {} 不是合法 JSON", path.display()))?;
    for name in &names {
        validate_name(name)?;
    }
    Ok(names)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocked_server_changes_are_persisted_and_subscribed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("blocked.json");
        let store = FederationBlocks::open(&path).unwrap();
        let mut updates = store.subscribe();
        store.set("bad.example.test", true).unwrap();
        assert!(updates.has_changed().unwrap());
        assert_eq!(
            *updates.borrow_and_update(),
            BTreeSet::from(["bad.example.test".into()])
        );
        let reopened = FederationBlocks::open(&path).unwrap();
        assert!(reopened.names().contains("bad.example.test"));
        store.set("bad.example.test", false).unwrap();
        assert!(
            !FederationBlocks::open(&path)
                .unwrap()
                .names()
                .contains("bad.example.test")
        );
        assert!(store.set("bad.*.test", true).is_err());
        assert!(store.names().is_empty());
    }

    /// 命令行工具和守护进程各有一份内存名单：写盘前重读时对方刚加进来的服务器要留下，不能按自己的旧值盖掉。
    #[test]
    fn a_second_writer_does_not_overwrite_the_first() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("blocked.json");
        let daemon = FederationBlocks::open(&path).unwrap();
        let command_line = FederationBlocks::open(&path).unwrap();
        daemon.set("first.example.test", true).unwrap();
        command_line.set("second.example.test", true).unwrap();
        assert_eq!(
            FederationBlocks::open(&path).unwrap().names(),
            BTreeSet::from([
                "first.example.test".to_owned(),
                "second.example.test".to_owned()
            ])
        );
        assert!(
            !daemon.names().contains("second.example.test"),
            "各自的内存名单"
        );
        daemon.reload().unwrap();
        assert!(daemon.names().contains("second.example.test"));
    }

    /// 内存还没刷到外部进程的删除：守护进程再把同一台加回去，磁盘也要写回，不能因为内存里已有就早退。
    #[test]
    fn stale_memory_does_not_skip_a_write() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("blocked.json");
        let daemon = FederationBlocks::open(&path).unwrap();
        daemon.set("bad.example.test", true).unwrap();
        FederationBlocks::open(&path)
            .unwrap()
            .set("bad.example.test", false)
            .unwrap();
        daemon.set("bad.example.test", true).unwrap();
        assert!(
            FederationBlocks::open(&path)
                .unwrap()
                .names()
                .contains("bad.example.test")
        );
    }
}
