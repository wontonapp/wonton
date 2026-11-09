//! 整台服务器的封禁独立于个人邀请过滤，变动与配对范围一起热加载。

use std::collections::BTreeSet;
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
        let updated = read(&self.path)?;
        if updated != *names {
            *names = updated.clone();
            self.changed.send_replace(updated);
        }
        Ok(())
    }

    pub fn set(&self, server: &str, blocked: bool) -> anyhow::Result<()> {
        super::routing::validate_name(server)?;
        let mut names = self.names.lock().unwrap();
        let mut updated = names.clone();
        if blocked {
            updated.insert(server.to_owned());
        } else {
            updated.remove(server);
        }
        if updated == *names {
            return Ok(());
        }
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        crate::install::write_private(&self.path, &serde_json::to_string(&updated)?)?;
        *names = updated.clone();
        self.changed.send_replace(updated);
        Ok(())
    }
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
        super::routing::validate_name(name)?;
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
}
