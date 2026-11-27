use std::collections::{BTreeMap, BTreeSet};
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use fs2::FileExt;
use serde::{Deserialize, Serialize};

use crate::process::Owner;

pub const STATE_FILE: &str = ".state.toml";
const LOCK_FILE: &str = ".state.toml.lock";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    #[default]
    InProgress,
    Done,
    Aborted,
    Recovered,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BatchStatus {
    #[default]
    Running,
    Closed,
    Aborted,
}

/// 一批 PR 及其 verify 时看到的 head。`Unknown` 的 head 存空串。
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Batch {
    #[serde(default)]
    pub prs: BTreeSet<u64>,
    #[serde(default)]
    pub heads: BTreeMap<u64, String>,
    #[serde(default)]
    pub status: BatchStatus,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Record {
    #[serde(default)]
    pub status: Status,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<Owner>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cycle: Option<String>,
    #[serde(default)]
    pub run_sha: String,
    #[serde(default)]
    pub heads: BTreeMap<u64, String>,
    #[serde(default)]
    pub closing: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resume_of: Option<String>,
    #[serde(default)]
    pub retries: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

impl Record {
    pub fn with_owner(cycle: &str, owner: &Owner) -> Self {
        Self {
            status: Status::InProgress,
            owner: Some(owner.clone()),
            cycle: Some(cycle.to_owned()),
            ..Self::default()
        }
    }

    pub fn cleared(&mut self) {
        self.owner = None;
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct State {
    #[serde(default)]
    pub version: u32,
    /// 当前作业所属进程（与 record.owner 同步），watch/compare_notes 靠它认人。
    #[serde(default, skip_serializing_if = "Option::is_none", with = "owner_text")]
    pub job: Option<Owner>,
    #[serde(default)]
    pub batches: BTreeMap<String, Batch>,
    #[serde(default)]
    pub records: BTreeMap<String, Record>,
    /// 最近开始的 cycle；BTreeMap 的顺序对 uuid 没意义，靠它找当前批次。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_cycle: Option<String>,
}

mod owner_text {
    use super::Owner;
    use serde::{Deserialize, Deserializer, Serializer, de::Error as _};

    pub fn serialize<S: Serializer>(value: &Option<Owner>, ser: S) -> Result<S::Ok, S::Error> {
        match value {
            Some(owner) => {
                let text = owner.to_string();
                ser.serialize_some(&text)
            }
            None => ser.serialize_none(),
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(de: D) -> Result<Option<Owner>, D::Error> {
        let text = Option::<String>::deserialize(de)?;
        text.map(|text| text.parse().map_err(D::Error::custom))
            .transpose()
    }
}

pub struct Board {
    root: PathBuf,
    state: State,
    _lock: Option<File>,
}

impl Board {
    /// 已有状态才打开；给 cancel/recover/compare_notes 用，不凭空建文件。
    pub fn open_existing(root: &Path) -> Result<Self> {
        let path = root.join(STATE_FILE);
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("没有状态文件 {}", path.display()))?;
        let state: State = toml::from_str(&text)
            .with_context(|| format!("状态文件解析失败 {}", path.display()))?;
        Ok(Self {
            root: root.to_owned(),
            state,
            _lock: None,
        })
    }

    /// 拿独占锁再打开；超时返回 `None`，调用方可以继续跑，只是写入可能互相覆盖。
    pub fn open_locked(root: &Path, timeout: Duration) -> Result<Option<Self>> {
        std::fs::create_dir_all(root)?;
        let lock = try_lock(root, timeout)?;
        let Some(lock) = lock else {
            return Ok(None);
        };
        Ok(Some(Self {
            root: root.to_owned(),
            state: read_state(root)?,
            _lock: Some(lock),
        }))
    }

    /// 不加锁按现有内容打开；没有文件就是空状态。
    pub fn open_or_default(root: &Path) -> Result<Self> {
        Ok(Self {
            root: root.to_owned(),
            state: read_state(root)?,
            _lock: None,
        })
    }

    pub fn state(&self) -> &State {
        &self.state
    }

    pub fn state_mut(&mut self) -> &mut State {
        &mut self.state
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn record(&self, cycle: &str) -> Option<&Record> {
        self.state.records.get(cycle)
    }

    pub fn batches(&self) -> impl Iterator<Item = (&String, &Batch)> {
        self.state.batches.iter()
    }

    /// 最新一批（start_cycle 记下的 last_cycle）。
    pub fn current_batch(&self) -> Option<(&String, &Batch)> {
        let cycle = self.state.last_cycle.as_ref()?;
        Some((cycle, self.state.batches.get(cycle)?))
    }

    pub fn in_progress(&self) -> impl Iterator<Item = (&String, &Record)> {
        self.state
            .records
            .iter()
            .filter(|(_, record)| record.status == Status::InProgress)
    }

    pub fn record_has_status(&self, status: Status) -> bool {
        self.state
            .records
            .values()
            .any(|record| record.status == status)
    }

    pub fn save(&self) -> Result<()> {
        let text = toml::to_string_pretty(&self.state)?;
        let path = self.root.join(STATE_FILE);
        let temp = self
            .root
            .join(format!(".state.toml.{}.tmp", std::process::id()));
        {
            let mut file = File::create(&temp)?;
            file.write_all(text.as_bytes())?;
            file.sync_all()?;
        }
        std::fs::rename(&temp, &path)?;
        Ok(())
    }

    pub fn start_cycle(&mut self, cycle: &str, prs: BTreeSet<u64>, owner: &Owner) -> Result<()> {
        self.state.version = 1;
        self.state.last_cycle = Some(cycle.to_owned());
        self.state.batches.insert(
            cycle.to_owned(),
            Batch {
                prs,
                ..Batch::default()
            },
        );
        self.state
            .records
            .insert(cycle.to_owned(), Record::with_owner(cycle, owner));
        self.state.job = Some(owner.clone());
        self.save()
    }

    /// 收尾与清 owner、清 job 在同一个写里；中途被 kill 不会留下半截状态。
    pub fn complete(&mut self, cycle: &str, closing: bool, reason: Option<&str>) -> Result<()> {
        if let Some(record) = self.state.records.get_mut(cycle) {
            record.status = Status::Done;
            record.closing = closing;
            record.cleared();
            record.reason = reason.map(str::to_owned);
        }
        if let Some(batch) = self.state.batches.get_mut(cycle) {
            batch.status = if closing {
                BatchStatus::Closed
            } else {
                BatchStatus::Aborted
            };
        }
        self.state.job = None;
        self.save()
    }

    /// 把死掉/无主的 in_progress 全部改成 recovered，并记下续跑来源。
    /// 活着的进程不碰；返回被收敛的 cycle。
    pub fn recover_in_progress(&mut self) -> Result<Vec<String>> {
        let mut recovered: Vec<String> = self
            .state
            .records
            .iter()
            .filter(|(_, record)| {
                record.status == Status::InProgress
                    && record.owner.as_ref().is_none_or(|owner| !owner.alive())
            })
            .map(|(cycle, _)| cycle.clone())
            .collect();
        if recovered.is_empty() {
            return Ok(recovered);
        }
        recovered.sort();
        for cycle in &recovered {
            let resume = cycle.clone();
            if let Some(record) = self.state.records.get_mut(cycle) {
                record.status = Status::Recovered;
                record.resume_of = Some(resume);
                record.cleared();
                record.reason = Some("recovered".to_owned());
            }
            if let Some(batch) = self.state.batches.get_mut(cycle) {
                batch.status = BatchStatus::Aborted;
            }
        }
        if self.state.job.as_ref().is_some_and(|owner| !owner.alive()) {
            self.state.job = None;
        }
        self.save()?;
        Ok(recovered)
    }

    /// 只改记录和批次；job 归调用方，多冲突时不能边遍历边清。
    pub fn abort_record(&mut self, cycle: &str, reason: &str) -> Result<()> {
        if let Some(record) = self.state.records.get_mut(cycle) {
            record.status = Status::Aborted;
            record.cleared();
            record.reason = Some(reason.to_owned());
        }
        if let Some(batch) = self.state.batches.get_mut(cycle) {
            batch.status = BatchStatus::Aborted;
        }
        self.save()
    }

    pub fn clear_job(&mut self) -> Result<()> {
        self.state.job = None;
        self.save()
    }
}

fn try_lock(root: &Path, timeout: Duration) -> Result<Option<File>> {
    let path = root.join(LOCK_FILE);
    let deadline = Instant::now() + timeout;
    loop {
        let mut file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&path)
            .with_context(|| format!("打开锁文件 {}", path.display()))?;
        match file.try_lock_exclusive() {
            Ok(()) => {
                file.set_len(0)?;
                if let Ok(owner) = Owner::current() {
                    file.write_all(owner.to_string().as_bytes())?;
                }
                return Ok(Some(file));
            }
            Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                if Instant::now() >= deadline {
                    return Ok(None);
                }
                std::thread::sleep(Duration::from_millis(25));
            }
            Err(err) => return Err(err.into()),
        }
    }
}

pub fn read_state(root: &Path) -> Result<State> {
    let path = root.join(STATE_FILE);
    match std::fs::read_to_string(&path) {
        Ok(text) => {
            toml::from_str(&text).with_context(|| format!("状态文件解析失败 {}", path.display()))
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(State::default()),
        Err(err) => Err(err.into()),
    }
}

pub fn lock_owner(root: &Path) -> Option<Owner> {
    std::fs::read_to_string(root.join(LOCK_FILE))
        .ok()?
        .split_whitespace()
        .next()?
        .parse()
        .ok()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Conflict {
    BatchWithoutOwner(String),
    OwnerWithoutBatch,
    NoBatch,
}

/// 分三次读：三类冲突来源不同步，一次拿全容易误报。
pub fn detect_conflicts(root: &Path) -> Result<Vec<Conflict>> {
    let mut conflicts = Vec::new();

    let first = Board::open_or_default(root)?;
    for (cycle, batch) in &first.state.batches {
        let owned = first
            .state
            .records
            .get(cycle)
            .is_some_and(|record| record.status == Status::InProgress);
        if batch.status == BatchStatus::Running && !owned {
            conflicts.push(Conflict::BatchWithoutOwner(cycle.clone()));
        }
    }

    let second = Board::open_or_default(root)?;
    let has_owner_batch = second.state.job.as_ref().is_some_and(|job| {
        second.state.batches.keys().any(|cycle| {
            second.state.records.get(cycle).is_some_and(|record| {
                record.status == Status::InProgress && record.owner.as_ref() == Some(job)
            })
        })
    });
    if second.state.job.is_some() && !has_owner_batch {
        conflicts.push(Conflict::OwnerWithoutBatch);
    }

    let third = Board::open_or_default(root)?;
    if third.state.batches.is_empty() {
        conflicts.push(Conflict::NoBatch);
    }

    Ok(conflicts)
}

/// 冲突按 owner 是否存活收敛：活着转交 json 交给下一次 `finish --recover`，
/// 死了就地清 job。都落到 aborted，不无限 failed。
pub fn reconcile_conflicts(root: &Path, board: &mut Board) -> Result<Vec<String>> {
    let conflicts = detect_conflicts(root)?;
    let job = board.state().job.clone();
    struct Pending {
        cycle: Option<String>,
        reason: &'static str,
    }

    // 先把每个冲突的 owner 快照下来再动状态，后面的冲突才读得到原 job。
    let mut pending = Vec::new();
    for conflict in conflicts {
        let (cycle, owner) = match &conflict {
            Conflict::BatchWithoutOwner(cycle) => (
                Some(cycle.clone()),
                board
                    .state
                    .records
                    .get(cycle)
                    .and_then(|record| record.owner.clone())
                    .or_else(|| job.clone()),
            ),
            Conflict::OwnerWithoutBatch | Conflict::NoBatch => {
                let cycle = board
                    .state
                    .records
                    .iter()
                    .rev()
                    .find(|(_, record)| record.status == Status::InProgress)
                    .map(|(cycle, _)| cycle.clone());
                (cycle, job.clone())
            }
        };
        let reason = if owner.as_ref().is_some_and(Owner::alive) {
            "aborted: owner alive→json"
        } else {
            "aborted: owner dead→job reconcile"
        };
        pending.push(Pending { cycle, reason });
    }

    let mut reasons = Vec::new();
    for item in pending {
        if let Some(cycle) = item.cycle {
            board.abort_record(&cycle, item.reason)?;
        }
        reasons.push(item.reason.to_owned());
    }
    // 死的 job 就地清；活的留着给下一次 --recover。
    if job.as_ref().is_some_and(|owner| !owner.alive()) {
        board.clear_job()?;
    } else if job.is_some() {
        board.save()?;
    }
    Ok(reasons)
}

pub fn require_existing(root: &Path) -> Result<Board> {
    let board = Board::open_existing(root)?;
    if board.state.records.is_empty() {
        bail!("状态文件里没有任何记录 {}", root.join(STATE_FILE).display());
    }
    Ok(board)
}
