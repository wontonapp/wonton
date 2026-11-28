mod common;

use std::collections::BTreeSet;
use std::time::Duration;

use common::{FakeApi, cycle, cycle_id};
use tempfile::TempDir;
use wonton_onboard::board::{Batch, Board, Record, Status};
use wonton_onboard::finish::{FinishRequest, Outcome, RetryDecision, decide_retry, finish};
use wonton_onboard::verify::{Guard, close_guard};

fn request(prs: &[u64]) -> FinishRequest {
    FinishRequest {
        recover: false,
        prs: prs.iter().copied().collect::<BTreeSet<_>>(),
    }
}

fn timeout() -> Duration {
    Duration::from_millis(50)
}

#[test]
fn same_batch_retries_within_cycle_without_details() {
    let dir = TempDir::new().unwrap();
    common::seed(dir.path(), cycle(&[7], "abc", Status::Done, None));
    let api = FakeApi::new().head(7, "abc");

    let outcome = finish(dir.path(), &request(&[7]), &api, timeout()).unwrap();
    assert!(
        matches!(
            outcome,
            Outcome::Closed {
                within_cycle: true,
                ..
            }
        ),
        "{outcome:?}"
    );
    assert_eq!(api.details_calls(7), 0);

    let board = Board::open_existing(dir.path()).unwrap();
    let record = board.record(&common::last_cycle(&board)).unwrap();
    assert_eq!(record.retries, 1);
    assert!(record.closing);
}

#[test]
fn new_pr_set_opens_new_cycle_and_verifies() {
    let dir = TempDir::new().unwrap();
    common::seed(dir.path(), cycle(&[7], "abc", Status::Done, None));
    let api = FakeApi::new().head(7, "abc").head(8, "def");

    let outcome = finish(dir.path(), &request(&[7, 8]), &api, timeout()).unwrap();
    let Outcome::Closed {
        cycle: new_cycle,
        within_cycle: false,
    } = outcome
    else {
        panic!("应当是新的 cycle 收尾");
    };
    assert_ne!(new_cycle, cycle_id());
    assert_eq!(api.details_calls(7), 1);
    assert_eq!(api.details_calls(8), 1);
}

#[test]
fn bumped_head_reruns_details_same_cycle() {
    let dir = TempDir::new().unwrap();
    common::seed(dir.path(), cycle(&[7], "abc", Status::Done, None));
    let api = FakeApi::new().head(7, "def");

    finish(dir.path(), &request(&[7]), &api, timeout()).unwrap();
    assert_eq!(api.details_calls(7), 1);

    let board = Board::open_existing(dir.path()).unwrap();
    let record = board.record(&common::last_cycle(&board)).unwrap();
    assert_eq!(record.run_sha, "def");
    assert!(record.closing);
}

#[test]
fn close_guard_unknown_skips_instead_of_mismatch() {
    assert_eq!(close_guard("", Some("abc")), Guard::Unknown);
    assert_eq!(close_guard("abc", Some("abc")), Guard::Ok);
    assert_eq!(
        close_guard("abc", Some("def")),
        Guard::Mismatch {
            live: "def".to_owned()
        }
    );
}

#[test]
fn retry_decision_only_for_same_batch_and_head() {
    let batch = Batch {
        prs: [7].into(),
        heads: [(7, "abc".to_owned())].into(),
        ..Batch::default()
    };
    let record = Record {
        status: Status::InProgress,
        run_sha: "abc".to_owned(),
        heads: [(7, "abc".to_owned())].into(),
        ..Record::default()
    };
    let same: BTreeSet<u64> = [7].into();
    assert_eq!(
        decide_retry(&record, &batch, &same),
        RetryDecision::RetryWithinCycle
    );

    let differ: BTreeSet<u64> = [7, 8].into();
    assert_eq!(
        decide_retry(&record, &batch, &differ),
        RetryDecision::OpenNewCycle
    );

    let unknown_head = Record {
        status: Status::InProgress,
        run_sha: "abc".to_owned(),
        ..Record::default()
    };
    assert_eq!(
        decide_retry(&unknown_head, &batch, &same),
        RetryDecision::OpenNewCycle
    );

    let done = Record {
        status: Status::Done,
        run_sha: "abc".to_owned(),
        heads: [(7, "abc".to_owned())].into(),
        ..Record::default()
    };
    assert_eq!(
        decide_retry(&done, &batch, &same),
        RetryDecision::RetryWithinCycle
    );

    let aborted = Record {
        status: Status::Aborted,
        run_sha: "abc".to_owned(),
        heads: [(7, "abc".to_owned())].into(),
        ..Record::default()
    };
    assert_eq!(
        decide_retry(&aborted, &batch, &same),
        RetryDecision::OpenNewCycle
    );
}
