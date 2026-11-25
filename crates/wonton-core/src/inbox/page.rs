//! 页面快照的 CSP 和 5 分钟短令牌（ADR-0119 第 4 节）。
//!
//! 页面是发送时的快照；CSP 固定从默认全断网出发，只把名单里的域名加进资源指令和 `connect-src`。
//! 名单为空就 `connect-src 'none'`，脚本、样式仍只收内联和同源。

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use wonton_protocol::MAX_INBOX_PAGE_BYTES;

use super::store::{Clock, InboxError};

/// 页面短令牌：令牌 →（条目 id，过期时间）。
#[derive(Clone, Default)]
pub struct PageTokens {
    inner: Arc<Mutex<HashMap<String, (String, u64)>>>,
}

impl PageTokens {
    pub fn issue(&self, id: &str, now: u64) -> String {
        let token = random_token();
        let mut inner = self.inner.lock().expect("页面令牌锁中毒");
        inner.retain(|_, (_, expires)| *expires > now);
        inner.insert(token.clone(), (id.to_owned(), now + 300_000));
        token
    }

    pub fn resolve(&self, token: &str, now: u64) -> Option<String> {
        let inner = self.inner.lock().expect("页面令牌锁中毒");
        inner
            .get(token)
            .filter(|(_, expires)| *expires > now)
            .map(|(id, _)| id.clone())
    }
}

fn random_token() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or_default();
    format!(
        "{:x}{:x}{:x}",
        nanos,
        COUNTER.fetch_add(1, Ordering::Relaxed),
        std::process::id()
    )
}

/// 按声明域名生成 CSP。域名只由 [`wonton_protocol::validate_network_domains`] 放行过的写法进来。
#[must_use]
pub fn policy(domains: &[String]) -> String {
    let https: Vec<&str> = domains
        .iter()
        .map(String::as_str)
        .filter(|d| d.starts_with("https://"))
        .collect();
    let connect: Vec<&str> = domains.iter().map(String::as_str).collect();
    let resources = if https.is_empty() {
        String::new()
    } else {
        format!(" {}", https.join(" "))
    };
    let connect = if connect.is_empty() {
        "'none'".to_owned()
    } else {
        connect.join(" ")
    };
    format!(
        "default-src 'none'; script-src 'self' 'unsafe-inline'{resources}; \
         style-src 'self' 'unsafe-inline'{resources}; img-src 'self' data:{resources}; \
         font-src 'self' data:{resources}; media-src 'self' data:{resources}; \
         connect-src {connect}; frame-src 'none'; base-uri 'none'; form-action 'none'"
    )
}

/// 页面快照写盘：0600。
pub fn write_private(path: &std::path::Path, bytes: &[u8]) -> Result<(), InboxError> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .mode(0o600)
        .open(path)
        .map_err(|e| InboxError::Io(e.to_string()))?;
    file.write_all(bytes)
        .map_err(|e| InboxError::Io(e.to_string()))
}

/// 先写同目录临时文件再 rename，崩溃不会留半条 JSON。临时名带进程号和本进程内的序号，
/// 同一进程并发写同一条目也不会互相截断。
pub fn write_atomic(path: &std::path::Path, bytes: &[u8]) -> Result<(), InboxError> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("page");
    let tmp = path.with_file_name(format!(
        "{name}.tmp.{}.{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    write_private(&tmp, bytes)?;
    std::fs::rename(&tmp, path).map_err(|e| InboxError::Io(e.to_string()))
}

pub fn validate_size(bytes: usize) -> Result<(), InboxError> {
    if bytes > MAX_INBOX_PAGE_BYTES {
        return Err(InboxError::PayloadTooLarge);
    }
    Ok(())
}

/// 判断 MIME 能不能当页面。
pub fn is_html(mime: Option<&str>) -> bool {
    mime.is_some_and(|mime| {
        let mime = mime.to_ascii_lowercase();
        mime == "text/html" || mime.starts_with("text/html;")
    })
}

/// 测试用时钟。
#[derive(Clone)]
pub struct FixedClock(pub Arc<std::sync::atomic::AtomicU64>);

impl FixedClock {
    pub fn new(now: u64) -> Self {
        Self(Arc::new(std::sync::atomic::AtomicU64::new(now)))
    }
    pub fn get(&self) -> u64 {
        self.0.load(std::sync::atomic::Ordering::Relaxed)
    }
    pub fn set(&self, now: u64) {
        self.0.store(now, std::sync::atomic::Ordering::Relaxed);
    }
}

impl Clock for FixedClock {
    fn now_ms(&self) -> u64 {
        self.0.load(std::sync::atomic::Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_domains_means_offline() {
        let csp = policy(&[]);
        assert!(csp.contains("connect-src 'none'"), "{csp}");
        assert!(!csp.contains("https://"), "{csp}");
    }

    #[test]
    fn only_declared_domains_are_opened() {
        let csp = policy(&[
            "https://cdn.example.com".into(),
            "wss://rt.example.com".into(),
        ]);
        assert!(csp.contains("connect-src https://cdn.example.com wss://rt.example.com"));
        assert!(csp.contains("script-src 'self' 'unsafe-inline' https://cdn.example.com"));
        assert!(!csp.contains("http://evil.example.com"));
        assert!(csp.contains("frame-src 'none'"));
    }

    #[test]
    fn a_page_over_one_mebibyte_is_rejected() {
        assert!(validate_size(MAX_INBOX_PAGE_BYTES).is_ok());
        assert!(matches!(
            validate_size(MAX_INBOX_PAGE_BYTES + 1),
            Err(InboxError::PayloadTooLarge)
        ));
    }
}

#[cfg(test)]
mod atomic_tests {
    use super::*;

    /// 同一进程并发写同一条目/同一页面：临时名各不相同，最后文件必须是完整的一份。
    #[test]
    fn concurrent_writes_do_not_collide() {
        for name in ["item.json", "page.html"] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join(name);
            std::thread::scope(|scope| {
                for i in 0..16u8 {
                    let path = &path;
                    scope.spawn(move || {
                        let body = vec![b'a' + i; 4096];
                        write_atomic(path, &body).unwrap();
                    });
                }
            });
            let written = std::fs::read(&path).unwrap();
            assert_eq!(written.len(), 4096);
            assert!(written.iter().all(|byte| *byte == written[0]));
            assert_eq!(
                std::fs::read_dir(dir.path()).unwrap().count(),
                1,
                "不留临时文件"
            );
        }
    }
}
