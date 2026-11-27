mod common;

use std::collections::BTreeSet;
use std::time::Duration;

use common::{FakeApi, cycle, cycle_id};
use tempfile::TempDir;
use wonton_onboard::board::{Batch, Board, Conflict, Status, detect_conflicts};
use wonton_onboard::finish::{FinishRequest, Outcome, cancel, finish};
use wonton_onboard::process::Owner;

#[test]
fn batch_without_owner_is_a_conflict() {
    let dir = TempDir::new().unwrap();
    let state = common::seed_state(
        dir.path(),
        wonton_onboard::board::State {
            batches: [("cycle-1".to_owned(), Batch::default())].into(),
            ..Default::default()
        },
    );
    state.save().unwrap();

    assert_eq!(
        detect_conflicts(dir.path()).unwrap(),
        vec![Conflict::BatchWithoutOwner("cycle-1".to_owned())]
    );
}

#[test]
fn owner_without_batch_is_a_conflict() {
    let dir = TempDir::new().unwrap();
    let state = common::seed_state(
        dir.path(),
        wonton_onboard::board::State {
            job: Some(common::dead_owner()),
            ..Default::default()
        },
    );
    state.save().unwrap();

    let conflicts = detect_conflicts(dir.path()).unwrap();
    assert!(
        conflicts.contains(&Conflict::OwnerWithoutBatch),
        "{conflicts:?}"
    );
}

#[test]
fn no_batch_is_a_conflict_on_empty_board() {
    let dir = TempDir::new().unwrap();
    common::seed(dir.path(), wonton_onboard::board::State::default());
    assert_eq!(
        detect_conflicts(dir.path()).unwrap(),
        vec![Conflict::NoBatch]
    );
}

#[test]
fn cancel_keeps_job_when_record_exists() {
    let dir = TempDir::new().unwrap();
    common::seed(
        dir.path(),
        cycle(&[7], "abc", Status::InProgress, Some(common::dead_owner())),
    );

    assert!(cancel(dir.path()).unwrap());
    let board = Board::open_existing(dir.path()).unwrap();
    assert!(board.state().job.is_some());
    assert_eq!(
        board.record(&cycle_id()).unwrap().status,
        Status::InProgress
    );
}

#[test]
fn cancel_clears_job_without_record() {
    let dir = TempDir::new().unwrap();
    common::seed(
        dir.path(),
        wonton_onboard::board::State {
            job: Some(common::dead_owner()),
            ..Default::default()
        },
    );

    assert!(!cancel(dir.path()).unwrap());
    let board = Board::open_existing(dir.path()).unwrap();
    assert!(board.state().job.is_none());
}

#[test]
fn reused_pid_with_other_start_does_not_block() {
    let dir = TempDir::new().unwrap();
    let mut owner = Owner::current().unwrap();
    owner.start += 1;
    common::seed(
        dir.path(),
        cycle(&[7], "abc", Status::InProgress, Some(owner)),
    );
    let api = FakeApi::new().head(7, "abc");

    let outcome = finish(
        dir.path(),
        &FinishRequest {
            recover: false,
            prs: [7].iter().copied().collect::<BTreeSet<_>>(),
        },
        &api,
        Duration::from_millis(50),
    )
    .unwrap();
    assert!(matches!(outcome, Outcome::Closed { .. }));
}
