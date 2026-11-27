mod common;

use std::collections::BTreeSet;
use std::time::Duration;

use common::{FakeApi, cycle, cycle_id, dead_owner};
use tempfile::TempDir;
use wonton_onboard::board::{Board, Status};
use wonton_onboard::finish::{FinishRequest, Outcome, finish, recover};
use wonton_onboard::process::Owner;
use wonton_onboard::verify::Notes;

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
fn set_failure_leaves_started_and_json_then_recovers() {
    let dir = TempDir::new().unwrap();
    let api = FakeApi::new().head(7, "abc");
    api.fail_set(7);

    let err = finish(dir.path(), &request(&[7]), &api, timeout()).unwrap_err();
    assert!(err.to_string().contains("PR_SET"), "{err}");

    let notes = Notes::load(dir.path()).unwrap();
    assert!(notes.prs[&7].started);
    assert!(!notes.prs[&7].set);

    let board = Board::open_existing(dir.path()).unwrap();
    let record = board.record(&common::last_cycle(&board)).unwrap();
    assert_eq!(record.status, Status::Done);
    assert!(record.owner.is_none());

    // 记录没清干净，普通 finish 不许自作主张继续。
    assert_eq!(
        finish(dir.path(), &request(&[7]), &api, timeout()).unwrap(),
        Outcome::JsonPending
    );

    api.fix();
    assert!(matches!(
        recover(dir.path(), &api).unwrap(),
        Outcome::Closed { .. }
    ));
    assert!(Notes::load(dir.path()).unwrap().prs.is_empty());
}

#[test]
fn dead_owner_in_progress_is_recovered() {
    let dir = TempDir::new().unwrap();
    common::seed(
        dir.path(),
        cycle(&[7], "abc", Status::InProgress, Some(dead_owner())),
    );
    Notes {
        cycle: cycle_id(),
        prs: [(
            7,
            wonton_onboard::verify::Note {
                head: Some("abc".to_owned()),
                started: true,
                set: false,
            },
        )]
        .into(),
    }
    .save(dir.path())
    .unwrap();

    let api = FakeApi::new().head(7, "abc");
    let outcome = recover(dir.path(), &api).unwrap();
    assert!(matches!(outcome, Outcome::Closed { .. }));

    let board = Board::open_existing(dir.path()).unwrap();
    let record = board.record(&common::last_cycle(&board)).unwrap();
    assert_eq!(record.status, Status::Done);
    assert!(record.closing);
    assert_eq!(record.resume_of.as_deref(), Some("cycle-1"));
    assert!(record.owner.is_none());
}

#[test]
fn live_owner_is_busy_and_untouched() {
    let dir = TempDir::new().unwrap();
    let owner = Owner::current().unwrap();
    common::seed(
        dir.path(),
        cycle(&[7], "abc", Status::InProgress, Some(owner.clone())),
    );
    let api = FakeApi::new().head(7, "abc");

    assert_eq!(
        finish(dir.path(), &request(&[7]), &api, timeout()).unwrap(),
        Outcome::Busy
    );
    let board = Board::open_existing(dir.path()).unwrap();
    assert_eq!(
        board.record(&common::last_cycle(&board)).unwrap().owner,
        Some(owner)
    );
    assert_eq!(api.details_calls(7), 0);
}

#[test]
fn failed_verify_keeps_json_for_next_recover() {
    let dir = TempDir::new().unwrap();
    let api = FakeApi::new().head(7, "abc");
    api.fail_details(7);

    assert!(finish(dir.path(), &request(&[7]), &api, timeout()).is_err());
    let notes = Notes::load(dir.path()).unwrap();
    assert!(notes.prs[&7].started);
    assert!(dir.path().join(".git/wonton-finish.json").exists());
}
