//! 锁屏上显不显示通知的内容（ADR-0101 §5）：整机设置，按通知类别分别开关，默认显示。
//!
//! 设备级设置。只有文件不存在才采用显示内容的初始值；损坏或读取失败时返回 Storage，不能重置隐私。

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tokio::sync::watch;
use wonton_l10n::l10n;
use wonton_protocol::{CoreError, NotificationCategory};

use crate::home::write_private;
use crate::l10n::{CATALOG, language};

const PRIVACY_FILE: &str = "lock-screen-content.json";

/// 哪些类别锁屏上隐藏内容。只存关掉的：没写过的类别显示内容。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Hidden {
    categories: Vec<NotificationCategory>,
}

impl Hidden {
    pub(crate) fn hidden(&self, category: NotificationCategory) -> bool {
        self.categories.contains(&category)
    }
}

pub(crate) struct LockScreenLink {
    path: PathBuf,
    view: watch::Sender<Hidden>,
    /// 写文件和发布新值是一步，否则并发时文件和状态可能停在不同的开关上。
    writing: Mutex<()>,
}

impl LockScreenLink {
    pub(crate) fn new(data_dir: &Path) -> Result<Self, CoreError> {
        let path = data_dir.join(PRIVACY_FILE);
        let storage_error = |error: String| CoreError::Storage {
            reason: l10n!(
                CATALOG,
                language(),
                "notification-lock-screen-storage-failed",
                path = path.display().to_string(),
                reason = error
            ),
        };
        let view = match fs::read(&path) {
            Ok(bytes) => {
                serde_json::from_slice(&bytes).map_err(|error| storage_error(error.to_string()))?
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Hidden::default(),
            Err(error) => return Err(storage_error(error.to_string())),
        };
        Ok(Self {
            path,
            view: watch::Sender::new(view),
            writing: Mutex::new(()),
        })
    }

    pub(crate) fn subscribe(&self) -> watch::Receiver<Hidden> {
        self.view.subscribe()
    }

    /// 写不了时返回 `Storage`，开关不动。
    pub(crate) fn set(
        &self,
        category: NotificationCategory,
        hidden: bool,
    ) -> Result<(), CoreError> {
        let _writing = self
            .writing
            .lock()
            .expect("the settings lock is never poisoned");
        let mut next = self.view.borrow().clone();
        let listed = next.categories.contains(&category);
        if hidden == listed {
            return Ok(());
        }
        if hidden {
            next.categories.push(category);
        } else {
            next.categories.retain(|listed| *listed != category);
        }
        let text =
            serde_json::to_vec(&next).expect("the settings are plain values and always serialize");
        write_private(&self.path, &text).map_err(|error| CoreError::Storage {
            reason: l10n!(
                CATALOG,
                language(),
                "notification-lock-screen-storage-failed",
                path = self.path.display().to_string(),
                reason = error.to_string()
            ),
        })?;
        self.view.send_replace(next);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_hidden_reads_as_content_shown() {
        let dir = tempfile::tempdir().unwrap();
        let link = LockScreenLink::new(dir.path()).unwrap();
        for category in NotificationCategory::ALL {
            assert!(!link.subscribe().borrow().hidden(category));
        }
    }

    #[test]
    fn a_written_switch_survives_a_new_link() {
        let dir = tempfile::tempdir().unwrap();
        LockScreenLink::new(dir.path())
            .unwrap()
            .set(NotificationCategory::Message, true)
            .unwrap();
        let link = LockScreenLink::new(dir.path()).unwrap();
        assert!(
            link.subscribe()
                .borrow()
                .hidden(NotificationCategory::Message)
        );
        assert!(
            !link
                .subscribe()
                .borrow()
                .hidden(NotificationCategory::Confirmation)
        );
        link.set(NotificationCategory::Message, false).unwrap();
        assert!(
            !LockScreenLink::new(dir.path())
                .unwrap()
                .subscribe()
                .borrow()
                .hidden(NotificationCategory::Message)
        );
    }

    #[test]
    fn corrupt_or_unreadable_privacy_never_defaults_to_showing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(PRIVACY_FILE);
        fs::write(&path, b"{broken").unwrap();
        assert!(matches!(
            LockScreenLink::new(dir.path()),
            Err(CoreError::Storage { .. })
        ));
        fs::remove_file(&path).unwrap();
        fs::create_dir(&path).unwrap();
        assert!(matches!(
            LockScreenLink::new(dir.path()),
            Err(CoreError::Storage { .. })
        ));
    }
}
