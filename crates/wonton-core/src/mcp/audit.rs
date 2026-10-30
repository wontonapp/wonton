//! 总线的审计日志：每次 `tools/call` 一行 JSON（ADR-0016），只追加，文件只有核心能读。

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::PathBuf;
use std::sync::Mutex;

use serde::Serialize;
use serde_json::Value;

/// 对一次调用的决定。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Decision {
    /// 不需要确认，直接执行。
    Allowed,
    /// 总线拒绝了（工具不存在、参数不对、不让 Bot 做的事）。
    Denied,
    /// 主人确认了。
    Approved,
    /// 主人拒绝了。
    Rejected,
    /// 主人没在期限内回答。
    Expired,
    /// 没能请主人确认（发不出卡片、等待的请求太多）。
    Unconfirmed,
}

#[derive(Serialize)]
pub struct Record<'a> {
    pub caller: &'a str,
    pub tool: &'a str,
    pub arguments: &'a Value,
    pub decision: Decision,
    /// 执行了的话：`ok` 或者失败的原因。
    pub outcome: Option<&'a str>,
}

pub struct Audit {
    path: PathBuf,
    file: Mutex<Option<File>>,
}

impl Audit {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            file: Mutex::new(None),
        }
    }

    /// 写一行。写不进去时记一条错误日志，不影响这次调用：审计日志坏了不该让主人用不了工具。
    pub fn record(&self, record: &Record<'_>) {
        let mut line = serde_json::json!({ "ts": crate::plugin::manager::now_secs() });
        if let (Some(line), Ok(Value::Object(fields))) =
            (line.as_object_mut(), serde_json::to_value(record))
        {
            line.extend(fields);
        }
        let mut file = self.file.lock().expect("审计日志锁中毒");
        let written = self.open(&mut file).and_then(|f| writeln!(f, "{line}"));
        if let Err(e) = written {
            tracing::error!(path = %self.path.display(), error = %e, "写审计日志失败");
            *file = None;
        }
    }

    fn open<'a>(&self, file: &'a mut Option<File>) -> std::io::Result<&'a mut File> {
        if file.is_none() {
            *file = Some(
                OpenOptions::new()
                    .create(true)
                    .append(true)
                    .mode(0o600)
                    .open(&self.path)?,
            );
        }
        Ok(file.as_mut().expect("刚打开"))
    }
}
