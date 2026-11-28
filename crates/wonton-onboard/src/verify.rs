use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result};

use crate::board::Board;

pub const NOTES_FILE: &str = "wonton-finish.json";

#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct Note {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub head: Option<String>,
    #[serde(default)]
    pub started: bool,
    #[serde(default)]
    pub set: bool,
}

#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct Notes {
    #[serde(default)]
    pub cycle: String,
    #[serde(default)]
    pub prs: BTreeMap<u64, Note>,
}

pub fn notes_path(root: &Path) -> PathBuf {
    let dot_git = root.join(".git");
    if dot_git.is_dir() {
        return dot_git.join(NOTES_FILE);
    }
    // linked worktree 的 `.git` 是文件，里面写着 gitdir。
    if let Ok(text) = std::fs::read_to_string(&dot_git)
        && let Some(path) = text.trim().strip_prefix("gitdir: ")
    {
        let dir = dot_git
            .parent()
            .map(|parent| parent.join(path))
            .unwrap_or_else(|| PathBuf::from(path));
        return dir.join(NOTES_FILE);
    }
    dot_git.join(NOTES_FILE)
}

impl Notes {
    pub fn load(root: &Path) -> Result<Self> {
        let path = notes_path(root);
        match std::fs::read_to_string(&path) {
            Ok(text) => serde_json::from_str(&text)
                .with_context(|| format!("收尾记录解析失败 {}", path.display())),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(err) => Err(err.into()),
        }
    }

    pub fn save(&self, root: &Path) -> Result<()> {
        let path = notes_path(root);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let temp = path.with_extension("json.tmp");
        std::fs::write(&temp, serde_json::to_vec_pretty(self)?)?;
        std::fs::rename(&temp, &path)?;
        Ok(())
    }

    pub fn clear(root: &Path) -> Result<()> {
        match std::fs::remove_file(notes_path(root)) {
            Ok(()) => Ok(()),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(err) => Err(err.into()),
        }
    }

    pub fn record_started(&mut self, pr: u64, head: Option<&str>) {
        let note = self.prs.entry(pr).or_default();
        note.started = true;
        note.head = head.map(str::to_owned);
    }

    pub fn record_set(&mut self, pr: u64) {
        self.prs.entry(pr).or_default().set = true;
    }
}

/// verify 需要的外部动作。测试用假实现，真实现走 gh。
pub trait Api {
    fn head(&self, pr: u64) -> Result<String>;
    fn details(&self, pr: u64, head: Option<&str>) -> Result<()>;
    /// PR_SET 落库；失败时记录停在 in_progress，等 recover。
    fn set(&self, pr: u64) -> Result<()>;
}

pub struct GhApi {
    root: std::path::PathBuf,
}

impl GhApi {
    pub fn new(root: &Path) -> Self {
        Self {
            root: root.to_owned(),
        }
    }
}

impl Api for GhApi {
    fn head(&self, pr: u64) -> Result<String> {
        let output = Command::new("gh")
            .current_dir(&self.root)
            .args([
                "pr",
                "view",
                &pr.to_string(),
                "--json",
                "headRefOid",
                "--jq",
                ".headRefOid",
            ])
            .output()
            .context("跑 gh 取 head 失败")?;
        if !output.status.success() {
            anyhow::bail!(
                "gh 取 PR {pr} 的 head 失败：{}",
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
    }

    fn set(&self, _pr: u64) -> Result<()> {
        Ok(())
    }

    fn details(&self, pr: u64, head: Option<&str>) -> Result<()> {
        let mut command = Command::new("gh");
        command.current_dir(&self.root);
        command.args([
            "pr",
            "view",
            &pr.to_string(),
            "--json",
            "number,title,headRefOid,mergeable,statusCheckRollup",
        ]);
        let filter = match head {
            Some(head) => format!(
                ". as $pr | {{pr: $pr.number, head: $pr.headRefOid, mergeable: $pr.mergeable, checks: [$pr.statusCheckRollup[] | {{name, conclusion}}]}} | select(.head == \"{head}\")"
            ),
            None => ". as $pr | {{pr: $pr.number, head: $pr.headRefOid, mergeable: $pr.mergeable}}"
                .to_owned(),
        };
        command.args(["--jq", &filter]);
        let output = command.output().context("跑 gh 取 PR 明细失败")?;
        if !output.status.success() {
            anyhow::bail!(
                "gh 取 PR {pr} 明细失败：{}",
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Guard {
    Ok,
    /// 没有 run_sha：不拿 PR 原 head 硬拗。
    Unknown,
    Mismatch {
        live: String,
    },
}

pub fn close_guard(run_sha: &str, live: Option<&str>) -> Guard {
    if run_sha.is_empty() {
        return Guard::Unknown;
    }
    match live {
        Some(live) if live == run_sha => Guard::Ok,
        Some(live) => Guard::Mismatch {
            live: live.to_owned(),
        },
        None => Guard::Unknown,
    }
}

/// 一个 cycle 的明细：先给每个 PR 落 started（带本次看到的 head），再跑明细，
/// 最后 PR_SET。任何一步失败就停，记录留在 in_progress 交给 recover。
pub fn verify_cycle(board: &mut Board, prs: &[u64], api: &dyn Api) -> Result<()> {
    let cycle = board
        .current_batch()
        .map(|(cycle, _)| cycle.clone())
        .context("没有可 verify 的批次")?;
    let mut notes = Notes {
        cycle: cycle.clone(),
        ..Notes::default()
    };
    notes.save(board.root())?;

    for pr in prs {
        let head = api.head(*pr).ok();
        notes.record_started(*pr, head.as_deref());
        notes.save(board.root())?;

        if let Some(head) = head.as_deref() {
            if let Some(record) = board.state_mut().records.get_mut(&cycle) {
                record.run_sha = head.to_owned();
                record.heads.insert(*pr, head.to_owned());
            }
            if let Some(batch) = board.state_mut().batches.get_mut(&cycle) {
                batch.heads.insert(*pr, head.to_owned());
            }
            board.save()?;
        }

        api.details(*pr, head.as_deref())?;
        api.set(*pr)?;
        notes.record_set(*pr);
        notes.save(board.root())?;
    }
    Ok(())
}

/// PR head 已经前进：换 run_sha 并只重跑明细，不换 cycle。
pub fn ensure_current(board: &mut Board, pr: u64, live: &str, api: &dyn Api) -> Result<()> {
    let cycle = board
        .current_batch()
        .map(|(cycle, _)| cycle.clone())
        .context("没有可续跑的批次")?;
    if let Some(record) = board.state_mut().records.get_mut(&cycle) {
        record.run_sha = live.to_owned();
        record.heads.insert(pr, live.to_owned());
    }
    if let Some(batch) = board.state_mut().batches.get_mut(&cycle) {
        batch.heads.insert(pr, live.to_owned());
    }
    board.save()?;
    let mut notes = Notes::load(board.root())?;
    notes.record_started(pr, Some(live));
    notes.save(board.root())?;
    api.details(pr, Some(live))
}
