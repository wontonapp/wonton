use std::path::Path;
use std::time::Duration;

use anyhow::Result;

use crate::board::{self, Board};
use crate::finish::{self, FinishRequest, Outcome};
use crate::process::Owner;
use crate::verify::{Api, Notes};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CompareOutcome {
    /// owner 活着，可以重试。
    Busy,
    Clean,
    Reconciled(Vec<String>),
}

/// 开跑前看一眼：owner 活着就跳过，死了才收敛。返回 true 表示本轮应跳过。
pub fn before_snapshot(root: &Path) -> Result<bool> {
    let mut board = Board::open_existing(root)?;
    if alive_owner(&board) {
        return Ok(true);
    }
    board.recover_in_progress()?;
    Ok(false)
}

/// 两条路（live 记录与收尾记录）都先看 owner，活着就报可重试的 busy；
/// 空/死才调整。顺序两边一致。
pub fn compare_notes(root: &Path) -> Result<CompareOutcome> {
    let mut board = Board::open_existing(root)?;
    if alive_owner(&board) {
        return Ok(CompareOutcome::Busy);
    }

    let notes = Notes::load(root)?;
    let mut reasons = board.recover_in_progress()?;
    let notes_stale = !notes.prs.is_empty() && board.in_progress().next().is_none();
    if notes_stale {
        let latest = board
            .state()
            .records
            .keys()
            .next_back()
            .cloned()
            .unwrap_or_else(|| notes.cycle.clone());
        let reason = "aborted: owner dead→job reconcile";
        board.abort_record(&latest, reason)?;
        reasons.push(reason.to_owned());
    }
    if reasons.is_empty() {
        Ok(CompareOutcome::Clean)
    } else {
        Ok(CompareOutcome::Reconciled(reasons))
    }
}

/// `every 5m` 的自动恢复：owner 活着写跳过；死了先收，再按原批次续跑。
pub fn autopilot(root: &Path, api: &dyn Api) -> Result<String> {
    if before_snapshot(root)? {
        return Ok("skipped: owner alive".to_owned());
    }
    let board = Board::open_existing(root)?;
    if let Some(cycle) = board.state().last_cycle.clone() {
        let prs = board
            .state()
            .batches
            .get(&cycle)
            .map(|batch| batch.prs.clone())
            .unwrap_or_default();
        if !prs.is_empty() {
            let request = FinishRequest { recover: true, prs };
            match finish::finish(root, &request, api, Duration::from_secs(5))? {
                Outcome::Closed { .. } => return Ok("recovered: closed".to_owned()),
                Outcome::Busy => return Ok("skipped: owner alive".to_owned()),
                Outcome::ConflictAborted(_) => return Ok("recovered: conflict aborted".to_owned()),
                Outcome::JsonPending => {}
            }
        }
    }
    Ok("clean".to_owned())
}

fn alive_owner(board: &Board) -> bool {
    board
        .in_progress()
        .any(|(_, record)| record.owner.as_ref().is_some_and(Owner::alive))
}

/// 给调用方一个能直接比对的状态名。
pub fn summary(root: &Path) -> Result<String> {
    if board::detect_conflicts(root)?.is_empty() {
        Ok("clean".to_owned())
    } else {
        Ok("conflict".to_owned())
    }
}
