#![allow(dead_code)]

use std::cell::RefCell;

use std::collections::{BTreeSet, HashMap};

use anyhow::{Result, bail};
use wonton_onboard::board::{Batch, Board, Record, State, Status};
use wonton_onboard::process::Owner;
use wonton_onboard::verify::Api;

#[derive(Default)]
pub struct FakeApi {
    heads: RefCell<HashMap<u64, String>>,
    fail_details: RefCell<Vec<u64>>,
    fail_set: RefCell<Vec<u64>>,
    details_calls: RefCell<Vec<u64>>,
    set_calls: RefCell<Vec<u64>>,
}

impl FakeApi {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn head(self, pr: u64, sha: &str) -> Self {
        self.heads.borrow_mut().insert(pr, sha.to_owned());
        self
    }

    pub fn set_head(&self, pr: u64, sha: &str) {
        self.heads.borrow_mut().insert(pr, sha.to_owned());
    }

    pub fn fail_details(&self, pr: u64) {
        self.fail_details.borrow_mut().push(pr);
    }

    pub fn fail_set(&self, pr: u64) {
        self.fail_set.borrow_mut().push(pr);
    }

    pub fn fix(&self) {
        self.fail_details.borrow_mut().clear();
        self.fail_set.borrow_mut().clear();
    }

    pub fn details_calls(&self, pr: u64) -> usize {
        self.details_calls
            .borrow()
            .iter()
            .filter(|called| **called == pr)
            .count()
    }

    pub fn set_calls(&self, pr: u64) -> usize {
        self.set_calls
            .borrow()
            .iter()
            .filter(|called| **called == pr)
            .count()
    }
}

impl Api for FakeApi {
    fn head(&self, pr: u64) -> Result<String> {
        self.heads
            .borrow()
            .get(&pr)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("假 API 没有 PR {pr} 的 head"))
    }

    fn details(&self, pr: u64, _head: Option<&str>) -> Result<()> {
        self.details_calls.borrow_mut().push(pr);
        if self.fail_details.borrow().contains(&pr) {
            bail!("明细失败");
        }
        Ok(())
    }

    fn set(&self, pr: u64) -> Result<()> {
        self.set_calls.borrow_mut().push(pr);
        if self.fail_set.borrow().contains(&pr) {
            bail!("PR_SET 失败");
        }
        Ok(())
    }
}

pub fn seed(root: &std::path::Path, state: State) {
    seed_state(root, state).save().unwrap();
}

pub fn seed_state(root: &std::path::Path, mut state: State) -> Board {
    state.version = 1;
    let mut board = Board::open_or_default(root).unwrap();
    *board.state_mut() = state;
    board
}

pub fn cycle(prs: &[u64], head: &str, status: Status, owner: Option<Owner>) -> State {
    let cycle = "cycle-1".to_owned();
    let mut state = State {
        last_cycle: Some(cycle.clone()),
        ..State::default()
    };
    state.batches.insert(
        cycle.clone(),
        Batch {
            prs: prs.iter().copied().collect::<BTreeSet<_>>(),
            heads: prs.iter().map(|pr| (*pr, head.to_owned())).collect(),
            status: if status == Status::InProgress {
                wonton_onboard::board::BatchStatus::Running
            } else {
                wonton_onboard::board::BatchStatus::Closed
            },
        },
    );
    state.records.insert(
        cycle.clone(),
        Record {
            status,
            owner: owner.clone(),
            cycle: Some(cycle),
            run_sha: head.to_owned(),
            heads: prs.iter().map(|pr| (*pr, head.to_owned())).collect(),
            ..Record::default()
        },
    );
    state.job = owner;
    state
}

pub fn dead_owner() -> Owner {
    Owner {
        pid: 4_000_000,
        start: 1,
    }
}

pub fn cycle_id() -> String {
    "cycle-1".to_owned()
}

pub fn last_cycle(board: &Board) -> String {
    board.state().last_cycle.clone().unwrap()
}
