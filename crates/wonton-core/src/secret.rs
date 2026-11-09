//! 随机令牌与比较。

use std::fs;
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

/// 256 位随机数的十六进制表示。
pub fn random_token() -> String {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).expect("操作系统随机数源不可用");
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// 长度相同时逐字节比较全部内容，避免按前缀泄露时序信息。
pub fn constant_time_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// 读取令牌文件；不存在时生成一个，权限 0600。
pub fn load_or_create(path: &Path) -> std::io::Result<String> {
    match fs::read_to_string(path) {
        Ok(text) if !text.trim().is_empty() => return Ok(text.trim().to_owned()),
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let token = random_token();
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(token.as_bytes())?;
    file.sync_all()?;
    Ok(token)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_are_random_and_persisted() {
        assert_ne!(random_token(), random_token());
        assert_eq!(random_token().len(), 64);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("core/owner-token");
        let first = load_or_create(&path).unwrap();
        assert_eq!(load_or_create(&path).unwrap(), first);
        assert!(constant_time_eq(&first, &first.clone()));
        assert!(!constant_time_eq(&first, &random_token()));
        assert!(!constant_time_eq("ab", "abc"));
    }
}
