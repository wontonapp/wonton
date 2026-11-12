//! 两个示例共用的小工具：私钥文件和日志初始化。

use std::{fs, io::Write, path::Path};

use anyhow::{Context, Result, bail};
use iroh::SecretKey;

/// 读取私钥文件；文件不存在时生成一把新私钥并写入（仅本人可读写），保证重启后 endpoint ID 不变。
pub fn load_or_create_key(path: &Path) -> Result<SecretKey> {
    match fs::read(path) {
        Ok(bytes) => {
            let Ok(bytes) = <[u8; 32]>::try_from(bytes.as_slice()) else {
                bail!("私钥文件 {} 不是 32 字节", path.display());
            };
            Ok(SecretKey::from_bytes(&bytes))
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            let key = SecretKey::generate();
            let mut options = fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
            let mut file = options
                .open(path)
                .with_context(|| format!("创建私钥文件 {}", path.display()))?;
            file.write_all(&key.to_bytes())
                .with_context(|| format!("写入私钥文件 {}", path.display()))?;
            eprintln!("已生成新私钥：{}", path.display());
            Ok(key)
        }
        Err(err) => Err(err).with_context(|| format!("读取私钥文件 {}", path.display())),
    }
}

/// 日志级别用 `RUST_LOG` 调，默认只看隧道自己的 info 日志。
pub fn init_tracing() {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("wonton_tunnel=info"));
    tracing_subscriber::fmt().with_env_filter(filter).init();
}
