//! 收新消息的方式（ADR-0003 的“设计：保持连接”）：存在本机的一个设置，做法同外观的设置文件
//! （`appearance.json`）。读不了时用默认值并记日志：方式选不了不该挡住 App 打开。
//!
//! 只在 `SetPushMethod` 成功落盘后才发布新值：换方式要连带删推送登记、启停前台服务，文件和状态不能停在两边。

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use tokio::sync::watch;
use wonton_l10n::l10n;
use wonton_protocol::{CoreError, PushMethod};

use crate::home::write_private;
use crate::l10n::{CATALOG, language};

const METHOD_FILE: &str = "push-method.json";

pub(crate) struct PushMethodLink {
    path: PathBuf,
    view: watch::Sender<PushMethod>,
    /// 写文件和发布新值是一步，否则并发时文件和状态可能停在不同的方式上。
    writing: Mutex<()>,
}

impl PushMethodLink {
    pub(crate) fn new(data_dir: &Path) -> Self {
        let path = data_dir.join(METHOD_FILE);
        let view = match read_method(&path) {
            Ok(method) => method,
            Err(reason) => {
                tracing::warn!("failed to read the push method, using the default: {reason}");
                PushMethod::default()
            }
        };
        Self {
            path,
            view: watch::Sender::new(view),
            writing: Mutex::new(()),
        }
    }

    pub(crate) fn subscribe(&self) -> watch::Receiver<PushMethod> {
        self.view.subscribe()
    }

    /// 写不了时返回 `Storage`，方式不变。
    pub(crate) fn set(&self, method: PushMethod) -> Result<(), CoreError> {
        let _writing = self
            .writing
            .lock()
            .expect("the settings lock is never poisoned");
        let text = serde_json::to_vec(&method)
            .expect("the push method is a plain enum and always serializes");
        write_private(&self.path, &text).map_err(|error| CoreError::Storage {
            reason: l10n!(
                CATALOG,
                language(),
                "push-method-write-failed",
                path = self.path.display().to_string(),
                reason = error.to_string()
            ),
        })?;
        self.view.send_replace(method);
        Ok(())
    }
}

/// 没选过时是默认值。返回的字符串只进日志（`new` 里记英文警告）。
fn read_method(path: &Path) -> Result<PushMethod, String> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(PushMethod::default());
        }
        Err(error) => return Err(format!("{}: {error}", path.display())),
    };
    serde_json::from_slice(&bytes)
        .map_err(|error| format!("{} is damaged: {error}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_file_reads_as_the_default() {
        let dir = tempfile::tempdir().unwrap();
        let link = PushMethodLink::new(dir.path());
        assert_eq!(*link.subscribe().borrow(), PushMethod::default());
    }

    #[test]
    fn a_written_method_survives_a_new_link() {
        let dir = tempfile::tempdir().unwrap();
        let link = PushMethodLink::new(dir.path());
        link.set(PushMethod::UnifiedPush).unwrap();
        let reopened = PushMethodLink::new(dir.path());
        assert_eq!(*reopened.subscribe().borrow(), PushMethod::UnifiedPush);
    }

    #[test]
    fn a_failed_write_keeps_the_old_value() {
        let dir = tempfile::tempdir().unwrap();
        // 设置文件的位置上放一个目录：读不了（用默认值），写也写不进去。
        std::fs::create_dir(dir.path().join(METHOD_FILE)).unwrap();
        let link = PushMethodLink::new(dir.path());
        assert!(matches!(
            link.set(PushMethod::UnifiedPush),
            Err(CoreError::Storage { .. })
        ));
        assert_eq!(*link.subscribe().borrow(), PushMethod::default());
    }
}
