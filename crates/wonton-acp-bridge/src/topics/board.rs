//! 话题工作板：一个房间里每个会话一行的状态表，加上给动态摘要用的工作板序号和变化记录。
//!
//! 持久化照 `state.rs` 的写法：先写临时文件再 rename；文件损坏报 `Error::Upstream`，
//! 不静默清空。时间一律是调用方传入的 Unix 毫秒，模块里不读系统时钟。

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::topics::SessionKey;
use crate::topics::claims;
use crate::topics::claims::Claim;

/// 话题名字取第一条消息的前 20 个字符（按字符不按字节，ADR-0058）。
pub(crate) const NAME_MAX_CHARS: usize = 20;
/// “在做什么”和结论里每行的截短长度。
pub(crate) const TEXT_MAX_CHARS: usize = 80;
/// 最近一轮的结论最多保留的行数（ADR-0058：最后回复的前 3 行）。
pub(crate) const SUMMARY_LINES: usize = 3;
/// 变化记录最多保留条数，超出丢最旧的。
const MAX_CHANGES: usize = 200;
/// 会话行最多保留条数，超出先丢最早的已结束行。
const MAX_ROWS: usize = 100;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum Status {
    Working,
    WaitingApproval,
    Idle,
    Finished,
    Queued,
}

impl Status {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Status::Working => "进行中",
            Status::WaitingApproval => "等你确认",
            Status::Idle => "空闲",
            Status::Finished => "已结束",
            Status::Queued => "排队中",
        }
    }

    /// 是否占话题名额：开着的话题（含空闲）占，排队和已结束的不占。
    pub(crate) fn occupies_slot(self) -> bool {
        matches!(
            self,
            Status::Working | Status::WaitingApproval | Status::Idle
        )
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct Row {
    pub(crate) key: SessionKey,
    pub(crate) name: String,
    pub(crate) doing: String,
    pub(crate) status: Status,
    pub(crate) summary: Vec<String>,
    pub(crate) files: Vec<String>,
    /// 本会话上一轮开始时的工作板序号；动态摘要从这里往后取。
    pub(crate) last_seen_seq: u64,
    /// 最近一次活动（轮开始或结束）的 Unix 毫秒，空闲结束按它判断。
    pub(crate) last_activity: u64,
    /// 排队中的完整首条消息，晋升时交回调用方去发送。
    pub(crate) pending_prompt: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct Change {
    pub(crate) seq: u64,
    pub(crate) by: SessionKey,
    pub(crate) text: String,
    /// 双方冲突这类记录，作者自己也要在自己的动态里看到。
    pub(crate) to_all: bool,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct TopicsDoc {
    #[serde(default)]
    seq: u64,
    #[serde(default)]
    rows: Vec<Row>,
    #[serde(default)]
    changes: Vec<Change>,
    #[serde(default)]
    claims: Vec<Claim>,
}

#[derive(Debug)]
pub(crate) struct Board {
    path: PathBuf,
    doc: TopicsDoc,
}

impl Board {
    pub(crate) async fn load(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_owned();
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|error| Error::Upstream(format!("创建桥话题工作板目录失败：{error}")))?;
        }
        let doc = match tokio::fs::read(&path).await {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map_err(|error| Error::Upstream(format!("桥话题工作板损坏：{error}")))?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => TopicsDoc::default(),
            Err(error) => return Err(Error::Upstream(format!("读取桥话题工作板失败：{error}"))),
        };
        Ok(Self { path, doc })
    }

    pub(crate) async fn persist(&self) -> Result<()> {
        let bytes = serde_json::to_vec_pretty(&self.doc)
            .map_err(|error| Error::Upstream(format!("编码桥话题工作板失败：{error}")))?;
        let temp = self.path.with_extension("tmp");
        tokio::fs::write(&temp, bytes)
            .await
            .map_err(|error| Error::Upstream(format!("写桥话题工作板失败：{error}")))?;
        tokio::fs::rename(&temp, &self.path)
            .await
            .map_err(|error| Error::Upstream(format!("替换桥话题工作板失败：{error}")))
    }

    pub(crate) fn seq(&self) -> u64 {
        self.doc.seq
    }

    pub(crate) fn row(&self, key: &SessionKey) -> Option<&Row> {
        self.doc.rows.iter().find(|row| &row.key == key)
    }

    pub(crate) fn row_mut(&mut self, key: &SessionKey) -> Option<&mut Row> {
        self.doc.rows.iter_mut().find(|row| &row.key == key)
    }

    pub(crate) fn rows(&self) -> &[Row] {
        &self.doc.rows
    }

    pub(crate) fn rows_mut(&mut self) -> &mut [Row] {
        &mut self.doc.rows
    }

    pub(crate) fn add_row(&mut self, row: Row) {
        self.doc.rows.push(row);
        while self.doc.rows.len() > MAX_ROWS {
            // 先丢最早的已结束行；行数远到不了上限时轮不到这个分支。
            let victim = self
                .doc
                .rows
                .iter()
                .position(|row| row.status == Status::Finished)
                .unwrap_or(0);
            self.doc.rows.remove(victim);
        }
    }

    pub(crate) fn log_change(&mut self, by: SessionKey, text: String, to_all: bool) {
        self.doc.seq += 1;
        let seq = self.doc.seq;
        self.doc.changes.push(Change {
            seq,
            by,
            text,
            to_all,
        });
        let excess = self.doc.changes.len().saturating_sub(MAX_CHANGES);
        self.doc.changes.drain(0..excess);
    }

    /// 给 viewer 看的变化：序号在 `after` 之后、作者不是 viewer（双方冲突除外），按先后。
    pub(crate) fn changes_for(&self, viewer: &SessionKey, after: u64) -> Vec<Change> {
        self.doc
            .changes
            .iter()
            .filter(|change| change.seq > after && (&change.by != viewer || change.to_all))
            .cloned()
            .collect()
    }

    /// 占名额的话题数（开着没结束的；排队的不占）。
    pub(crate) fn active_topics(&self) -> usize {
        self.doc
            .rows
            .iter()
            .filter(|row| row.key.is_topic() && row.status.occupies_slot())
            .count()
    }

    /// 排队中的话题，按排队先后。
    pub(crate) fn queued_keys(&self) -> Vec<SessionKey> {
        self.doc
            .rows
            .iter()
            .filter(|row| row.status == Status::Queued)
            .map(|row| row.key.clone())
            .collect()
    }

    /// 只读访问改文件表。
    pub(crate) fn claims(&self) -> &[Claim] {
        &self.doc.claims
    }

    pub(crate) fn record_claims(&mut self, key: &SessionKey, paths: &[String], now: u64) {
        claims::record(&mut self.doc.claims, key, paths, now);
    }

    pub(crate) fn seal_claims_of(&mut self, key: &SessionKey, now: u64) {
        claims::seal_turn(&mut self.doc.claims, key, now);
    }

    pub(crate) fn seal_all_claims(&mut self, now: u64) {
        claims::seal_after_load(&mut self.doc.claims, now);
    }
}
