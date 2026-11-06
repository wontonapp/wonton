//! 本机存储：设备密钥在壳给的数据目录里，所有服务器共用；上次的连接信息在这台服务器自己的目录里（ADR-0042）。
//!
//! 设备密钥决定本机的 endpoint ID，换了就要重新配对，所以第一次启动生成后一直保留。M0 存成文件（仅本人
//! 可读写）；上架前换成系统钥匙串（iOS Keychain、Android Keystore），并且不进备份，避免恢复到另一台手机后
//! 两台设备共用一个身份。

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::str::FromStr;

use iroh::SecretKey;
use wonton_protocol::CoreError;

const DEVICE_KEY: &str = "device-key";
const TICKET: &str = "home-ticket.json";

pub(crate) struct Storage {
    root: PathBuf,
    dir: PathBuf,
}

impl Storage {
    /// `root` 是壳给的数据目录，`dir` 是这台服务器的目录。
    pub(crate) fn open(root: &Path, dir: &Path) -> Result<Self, CoreError> {
        for dir in [root, dir] {
            fs::create_dir_all(dir).map_err(|e| storage_error(dir, &e))?;
        }
        Ok(Self {
            root: root.to_owned(),
            dir: dir.to_owned(),
        })
    }

    /// 读取设备密钥；第一次启动时生成并保存。
    pub(crate) fn device_key(&self) -> Result<SecretKey, CoreError> {
        let path = self.root.join(DEVICE_KEY);
        match fs::read_to_string(&path) {
            Ok(text) => SecretKey::from_str(text.trim()).map_err(|e| CoreError::Storage {
                reason: format!("设备密钥 {} 已损坏：{e}", path.display()),
            }),
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                let key = SecretKey::generate();
                let hex: String = key.to_bytes().iter().map(|b| format!("{b:02x}")).collect();
                write_private(&path, hex.as_bytes()).map_err(|e| storage_error(&path, &e))?;
                Ok(key)
            }
            Err(e) => Err(storage_error(&path, &e)),
        }
    }

    /// 上次保存的连接信息。
    pub(crate) fn ticket(&self) -> Result<Option<String>, CoreError> {
        let path = self.dir.join(TICKET);
        match fs::read_to_string(&path) {
            Ok(text) => Ok(Some(text)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(storage_error(&path, &e)),
        }
    }

    pub(crate) fn save_ticket(&self, ticket: &str) -> Result<(), CoreError> {
        let path = self.dir.join(TICKET);
        write_private(&path, ticket.as_bytes()).map_err(|e| storage_error(&path, &e))
    }
}

/// 先写临时文件再改名，权限仅本人可读写。
pub(crate) fn write_private(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let tmp = path.with_extension("tmp");
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let mut file = options.open(&tmp)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    fs::rename(&tmp, path)
}

fn storage_error(path: &Path, error: &io::Error) -> CoreError {
    CoreError::Storage {
        reason: format!("{}：{error}", path.display()),
    }
}
