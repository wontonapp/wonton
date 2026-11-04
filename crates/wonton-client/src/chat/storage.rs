//! 聊天在本机存的东西，都在壳给的数据目录的 `matrix/` 下：
//!
//! - `session.json`：登录状态（用户、设备、访问令牌和刷新令牌）、homeserver 地址和它属于哪台服务器，重启后据此恢复
//!   会话；
//! - `store/`：matrix-sdk 的 SQLite 存储，设备密钥、交叉签名、房间密钥、消息和媒体缓存；
//! - `recovery-key`：刚设置恢复时生成的恢复密钥，只留到主人确认已经另外保存为止（App 在那之前被关掉，
//!   下次启动还能看到）。
//!
//! 和设备密钥一样，M0 存成仅本人可读写的文件，SQLite 存储也不加口令（口令放在同一个目录里没有意义），靠系统
//! 对 App 数据的保护；上架前和设备密钥一起换成系统钥匙串（iOS Keychain、Android Keystore）。

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use wonton_matrix::SavedSession;
use wonton_protocol::{CoreError, SecretString};

use crate::home::write_private;

const DIR: &str = "matrix";
const SESSION: &str = "session.json";
const STORE: &str = "store";
const RECOVERY_KEY: &str = "recovery-key";

/// 存下来的会话：登录状态加上登录时的服务器。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct StoredSession {
    /// 登录到哪台服务器：隧道的 endpoint ID，换 IP、换中继都不变。homeserver 地址和用户认不出服务器：每台默认都是
    /// `matrix.wonton.localhost` 上的 `@owner`。没有这个字段的文件读出来是空串，对不上任何服务器，重新登录一次。
    #[serde(default)]
    pub(crate) server_id: String,
    pub(crate) homeserver_url: String,
    pub(crate) session: SavedSession,
}

pub(crate) struct ChatStorage {
    dir: PathBuf,
}

impl ChatStorage {
    pub(crate) fn open(data_dir: &Path) -> Result<Self, CoreError> {
        let dir = data_dir.join(DIR);
        fs::create_dir_all(&dir).map_err(|e| storage_error(&dir, &e))?;
        Ok(Self { dir })
    }

    /// matrix-sdk 的 SQLite 存储目录。
    pub(crate) fn store_dir(&self) -> PathBuf {
        self.dir.join(STORE)
    }

    /// SQLite 存储还在不在。登录状态要和它一起用：存储没了还恢复，SDK 会给已有的设备 ID 生成新的设备密钥。
    pub(crate) fn has_store(&self) -> bool {
        self.store_dir().is_dir()
    }

    pub(crate) fn session(&self) -> Result<Option<StoredSession>, CoreError> {
        let path = self.dir.join(SESSION);
        match fs::read_to_string(&path) {
            Ok(text) => serde_json::from_str(&text)
                .map(Some)
                .map_err(|e| CoreError::Storage {
                    reason: format!("聊天的登录状态 {} 已损坏：{e}", path.display()),
                }),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(storage_error(&path, &e)),
        }
    }

    pub(crate) fn save_session(&self, session: &StoredSession) -> Result<(), CoreError> {
        let path = self.dir.join(SESSION);
        let text = serde_json::to_string(session).expect("登录状态总能编成 JSON");
        write_private(&path, text.as_bytes()).map_err(|e| storage_error(&path, &e))
    }

    /// 删掉登录状态和本地存储：设备被吊销、换了服务器，或者存储和登录状态对不上时，旧的设备密钥和房间密钥都用不上了。
    /// 还没确认保存的恢复密钥留着，主人在新设备上还要用它。
    pub(crate) fn forget_session(&self) -> Result<(), CoreError> {
        let session = self.dir.join(SESSION);
        match fs::remove_file(&session) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(storage_error(&session, &e)),
        }
        let store = self.store_dir();
        match fs::remove_dir_all(&store) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(storage_error(&store, &e)),
        }
    }

    /// 还没确认保存的恢复密钥。
    pub(crate) fn recovery_key(&self) -> Result<Option<SecretString>, CoreError> {
        let path = self.dir.join(RECOVERY_KEY);
        match fs::read_to_string(&path) {
            Ok(text) => Ok(Some(SecretString::new(text.trim()))),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(storage_error(&path, &e)),
        }
    }

    pub(crate) fn save_recovery_key(&self, key: &SecretString) -> Result<(), CoreError> {
        let path = self.dir.join(RECOVERY_KEY);
        write_private(&path, key.expose().as_bytes()).map_err(|e| storage_error(&path, &e))
    }

    /// 主人确认已经另外保存了恢复密钥：本机不再留着。
    pub(crate) fn forget_recovery_key(&self) -> Result<(), CoreError> {
        let path = self.dir.join(RECOVERY_KEY);
        match fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(storage_error(&path, &e)),
        }
    }
}

fn storage_error(path: &Path, error: &io::Error) -> CoreError {
    CoreError::Storage {
        reason: format!("{}：{error}", path.display()),
    }
}
