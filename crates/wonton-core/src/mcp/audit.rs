//! 审计日志（`core/audit.jsonl`）：总线每次 `tools/call` 一行 JSON（ADR-0016），文件服务的下载、上传、删除、授权也写进来
//! （ADR-0020）。只追加，文件只有核心能读。总线和文件服务共用一个 [`Audit`]：一行一次写入，彼此不会交错。

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::PathBuf;
use std::sync::Mutex;

use serde::Serialize;
use serde_json::Value;
use wonton_protocol::{FileFailure, FileSource};

use crate::files::record::{FileRecord, Origin};
use crate::plugin::PluginId;

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
    /// 这次调用碰到的文件（ADR-0020 的“审计”）。
    #[serde(skip_serializing_if = "<[_]>::is_empty")]
    pub files: &'a [AuditedFile],
}

/// 审计里的一个文件：编号、来源插件和引用。登记不了的没有编号，写明类别。
#[derive(Debug, Clone, Serialize)]
pub struct AuditedFile {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plugin: Option<String>,
    pub reference: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refused: Option<&'static str>,
}

impl AuditedFile {
    pub fn of(record: &FileRecord) -> Self {
        let (plugin, reference) = match &record.origin {
            Origin::Share { reference } | Origin::Upload { reference, .. } => (
                None,
                format!("{}/{}", reference.share, reference.path.join("/")),
            ),
            Origin::Plugin { plugin, uri, .. } => (Some(plugin.to_string()), uri.clone()),
        };
        let plugin = plugin.or_else(|| match &record.source {
            FileSource::Bot { plugin, .. } => Some(plugin.clone()),
            _ => None,
        });
        Self {
            id: Some(record.id.to_string()),
            plugin,
            reference,
            refused: None,
        }
    }

    /// 插件给了引用，登记不了。
    pub fn refused(plugin: &PluginId, uri: &str, kind: FileFailure) -> Self {
        Self {
            id: None,
            plugin: Some(plugin.to_string()),
            reference: uri.to_owned(),
            refused: Some(kind.as_str()),
        }
    }
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
    pub fn record(&self, record: &impl Serialize) {
        let mut line = serde_json::json!({ "ts": crate::plugin::manager::now_secs() });
        if let (Some(line), Ok(Value::Object(fields))) =
            (line.as_object_mut(), serde_json::to_value(record))
        {
            line.extend(fields);
        }
        let mut file = self.file.lock().expect("审计日志锁中毒");
        let written = self
            .open(&mut file)
            .and_then(|f| f.write_all(format!("{line}\n").as_bytes()));
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
