use std::collections::BTreeSet;
use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use uuid::Uuid;

use crate::board::{self, Batch, Board, Conflict, Record, Status, reconcile_conflicts};
use crate::process::Owner;
use crate::verify::{self, Api, Notes};

/// 取消：record 存在时保留 job，交给下一次 recover；没有记录才清 job。
pub fn cancel(root: &Path) -> Result<bool> {
    let mut board = Board::open_existing(root)?;
    if board.state().records.is_empty() {
        board.clear_job()?;
        return Ok(false);
    }
    Ok(true)
}

/// 显式恢复：按收尾记录或最后一批续跑，带上 `recover` 放行 json 门。
pub fn recover(root: &Path, api: &dyn Api) -> Result<Outcome> {
    let board = Board::open_existing(root)?;
    let notes = Notes::load(root)?;
    let prs = notes.prs.keys().copied().collect::<BTreeSet<u64>>();
    let prs = if prs.is_empty() {
        board
            .state()
            .last_cycle
            .as_ref()
            .and_then(|cycle| board.state().batches.get(cycle))
            .map(|batch| batch.prs.clone())
            .unwrap_or_default()
    } else {
        prs
    };
    finish(
        root,
        &FinishRequest { recover: true, prs },
        api,
        LOCK_TIMEOUT,
    )
}

pub const LOCK_TIMEOUT: Duration = Duration::from_secs(5);

pub struct FinishRequest {
    pub recover: bool,
    pub prs: BTreeSet<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// 另一个活着的进程正在跑这一批。
    Busy,
    /// `.git/wonton-finish.json` 还在，先 `finish --recover`。
    JsonPending,
    /// 冲突里有活着的 owner，已写 json 留给下一次 `--recover`。
    ConflictAborted(Vec<String>),
    Closed {
        cycle: String,
        within_cycle: bool,
    },
}

/// 只有“没有新一批”的原地重试才允许直接放行；新 cycle 永远要重验明细。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RetryDecision {
    RetryWithinCycle,
    OpenNewCycle,
}

pub fn decide_retry(record: &Record, batch: &Batch, prs: &BTreeSet<u64>) -> RetryDecision {
    let same_prs = prs == &batch.prs;
    let same_heads = !record.heads.is_empty()
        && prs
            .iter()
            .all(|pr| record.heads.get(pr) == batch.heads.get(pr));
    if matches!(record.status, Status::InProgress | Status::Done) && same_prs && same_heads {
        RetryDecision::RetryWithinCycle
    } else {
        RetryDecision::OpenNewCycle
    }
}

pub fn finish(
    root: &Path,
    request: &FinishRequest,
    api: &dyn Api,
    lock_timeout: Duration,
) -> Result<Outcome> {
    let notes = Notes::load(root)?;
    let notes_pending = !notes.prs.is_empty() || verify::notes_path(root).exists();
    if notes_pending && !request.recover {
        return Ok(Outcome::JsonPending);
    }

    let mut board = match Board::open_locked(root, lock_timeout)? {
        Some(board) => board,
        None => Board::open_or_default(root)?,
    };

    if board
        .in_progress()
        .any(|(_, record)| record.owner.as_ref().is_some_and(Owner::alive))
    {
        return Ok(Outcome::Busy);
    }

    let conflicts = board::detect_conflicts(root)?;
    if !conflicts.is_empty() && !is_first_run(&board, &conflicts) {
        let reasons = reconcile_conflicts(root, &mut board)?;
        if reasons.iter().any(|reason| reason.contains("alive")) {
            return Ok(Outcome::ConflictAborted(reasons));
        }
    }

    let mut recovered = board.recover_in_progress()?;
    if recovered.is_empty()
        && request.recover
        && !notes.prs.is_empty()
        && let Some(cycle) = pending_cycle(&board, &notes)
    {
        recovered.push(cycle);
    }
    let owner = Owner::current()?;
    let (cycle, prs, within_cycle) = claim(&mut board, &recovered, request, &owner)?;

    let result = run_claimed(root, &mut board, &cycle, &prs, within_cycle, api);
    if let Err(err) = &result {
        let _ = board.complete(&cycle, false, Some(&format!("failed: {err}")));
    }
    result
}

fn is_first_run(board: &Board, conflicts: &[Conflict]) -> bool {
    board.state().batches.is_empty()
        && board.state().records.is_empty()
        && conflicts
            .iter()
            .all(|conflict| *conflict == Conflict::NoBatch)
}

fn pending_cycle(board: &Board, notes: &Notes) -> Option<String> {
    if board.state().batches.contains_key(&notes.cycle) {
        return Some(notes.cycle.clone());
    }
    board.state().last_cycle.clone()
}

fn claim(
    board: &mut Board,
    recovered: &[String],
    request: &FinishRequest,
    owner: &Owner,
) -> Result<(String, BTreeSet<u64>, bool)> {
    if let Some(cycle) = recovered.last().cloned() {
        let prs = board
            .state()
            .batches
            .get(&cycle)
            .map(|batch| batch.prs.clone())
            .unwrap_or_default();
        reown(board, &cycle, owner)?;
        return Ok((cycle, prs, false));
    }

    let decision = match board.current_batch() {
        Some((cycle, batch)) => {
            let record = board
                .state()
                .records
                .get(cycle)
                .cloned()
                .unwrap_or_default();
            decide_retry(&record, batch, &request.prs)
        }
        None => RetryDecision::OpenNewCycle,
    };

    if decision == RetryDecision::RetryWithinCycle {
        let cycle = board
            .current_batch()
            .map(|(cycle, _)| cycle.clone())
            .context("重试时没有当前批次")?;
        let prs = request.prs.clone();
        if let Some(record) = board.state_mut().records.get_mut(&cycle) {
            record.retries += 1;
        }
        board.state_mut().job = Some(owner.clone());
        board.save()?;
        Ok((cycle, prs, true))
    } else {
        if request.prs.is_empty() {
            bail!("新一批 finish 需要至少一个 PR");
        }
        let cycle = Uuid::new_v4().to_string();
        board.start_cycle(&cycle, request.prs.clone(), owner)?;
        Ok((cycle, request.prs.clone(), false))
    }
}

fn reown(board: &mut Board, cycle: &str, owner: &Owner) -> Result<()> {
    if let Some(record) = board.state_mut().records.get_mut(cycle) {
        record.status = Status::InProgress;
        record.owner = Some(owner.clone());
        record.resume_of = Some(cycle.to_owned());
    }
    board.state_mut().job = Some(owner.clone());
    board.state_mut().last_cycle = Some(cycle.to_owned());
    board.save()
}

fn run_claimed(
    root: &Path,
    board: &mut Board,
    cycle: &str,
    prs: &BTreeSet<u64>,
    within_cycle: bool,
    api: &dyn Api,
) -> Result<Outcome> {
    let prs: Vec<u64> = prs.iter().copied().collect();
    if within_cycle {
        Notes {
            cycle: cycle.to_owned(),
            ..Notes::default()
        }
        .save(root)?;
    } else {
        verify::verify_cycle(board, &prs, api)?;
    }

    for pr in &prs {
        let run_sha = board
            .record(cycle)
            .and_then(|record| record.heads.get(pr).or(Some(&record.run_sha)))
            .cloned()
            .unwrap_or_default();
        let live = api.head(*pr).ok();
        if let verify::Guard::Mismatch { live } = verify::close_guard(&run_sha, live.as_deref()) {
            verify::ensure_current(board, *pr, &live, api)?;
        }
    }

    board.complete(cycle, true, None)?;
    Notes::clear(root)?;
    Ok(Outcome::Closed {
        cycle: cycle.to_owned(),
        within_cycle,
    })
}
