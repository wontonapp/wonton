mod common;

use std::collections::BTreeSet;
use std::time::Duration;

use common::{FakeApi, dead_owner};
use tempfile::TempDir;
use wonton_onboard::board::{Batch, BatchStatus, Board, Record, State, Status};
use wonton_onboard::finish::{Outcome, finish, recover};
use wonton_onboard::process::Owner;
use wonton_onboard::verify::Notes;
use wonton_onboard::watch::{CompareOutcome, compare_notes};

fn record(cycle: &str, head: &str, status: Status, owner: Option<Owner>) -> Record {
    Record {
        status,
        owner,
        cycle: Some(cycle.to_owned()),
        run_sha: head.to_owned(),
        heads: [(7, head.to_owned())].into(),
        ..Record::default()
    }
}

fn batch(prs: &[u64], head: &str, status: BatchStatus) -> Batch {
    Batch {
        prs: prs.iter().copied().collect::<BTreeSet<_>>(),
        heads: prs.iter().map(|pr| (*pr, head.to_owned())).collect(),
        status,
    }
}

fn two_dead_records() -> State {
    let mut state = State {
        last_cycle: Some("0000-new".to_owned()),
        ..State::default()
    };
    state.batches.insert(
        "0000-new".to_owned(),
        batch(&[8], "new-head", BatchStatus::Running),
    );
    state.records.insert(
        "0000-new".to_owned(),
        record(
            "0000-new",
            "new-head",
            Status::InProgress,
            Some(dead_owner()),
        ),
    );
    state.batches.insert(
        "zzzz-old".to_owned(),
        batch(&[9], "old-head", BatchStatus::Running),
    );
    state.records.insert(
        "zzzz-old".to_owned(),
        record(
            "zzzz-old",
            "old-head",
            Status::InProgress,
            Some(dead_owner()),
        ),
    );
    state.job = Some(dead_owner());
    state
}

fn notes_for(cycle: &str, pr: u64, head: &str) -> Notes {
    Notes {
        cycle: cycle.to_owned(),
        prs: [(
            pr,
            wonton_onboard::verify::Note {
                head: Some(head.to_owned()),
                started: true,
                set: false,
            },
        )]
        .into(),
    }
}

#[test]
fn recover_prefers_notes_cycle_over_uuid_order() {
    let dir = TempDir::new().unwrap();
    common::seed(dir.path(), two_dead_records());
    notes_for("0000-new", 8, "new-head")
        .save(dir.path())
        .unwrap();
    let api = FakeApi::new().head(8, "new-head").head(9, "old-head");

    let outcome = recover(dir.path(), &api).unwrap();
    let Outcome::Closed { cycle, .. } = outcome else {
        panic!("应当收 json 指的那条");
    };
    assert_eq!(cycle, "0000-new");

    // 另一条还没收，json 要留着。
    assert!(dir.path().join(".git/wonton-finish.json").exists());
    let board = Board::open_existing(dir.path()).unwrap();
    assert_eq!(board.record("zzzz-old").unwrap().status, Status::Recovered);

    let outcome = recover(dir.path(), &api).unwrap();
    assert!(matches!(outcome, Outcome::Closed { .. }));
    assert!(!dir.path().join(".git/wonton-finish.json").exists());
}

#[test]
fn compare_notes_does_not_flip_finished_record() {
    let dir = TempDir::new().unwrap();
    let mut state = State {
        last_cycle: Some("z-done".to_owned()),
        ..State::default()
    };
    state.batches.insert(
        "a-dead".to_owned(),
        batch(&[8], "dead-head", BatchStatus::Running),
    );
    state.records.insert(
        "a-dead".to_owned(),
        record(
            "a-dead",
            "dead-head",
            Status::InProgress,
            Some(dead_owner()),
        ),
    );
    state.batches.insert(
        "z-done".to_owned(),
        batch(&[9], "done-head", BatchStatus::Closed),
    );
    state.records.insert(
        "z-done".to_owned(),
        record("z-done", "done-head", Status::Done, None),
    );
    state.job = Some(dead_owner());
    common::seed(dir.path(), state);
    notes_for("a-dead", 8, "dead-head")
        .save(dir.path())
        .unwrap();

    assert!(matches!(
        compare_notes(dir.path()).unwrap(),
        CompareOutcome::Reconciled(_)
    ));
    let board = Board::open_existing(dir.path()).unwrap();
    assert_eq!(board.record("z-done").unwrap().status, Status::Done);
    assert_eq!(
        board
            .batches()
            .find(|(cycle, _)| *cycle == "z-done")
            .unwrap()
            .1
            .status,
        BatchStatus::Closed
    );
}

#[test]
fn alive_conflict_writes_json_and_keeps_job() {
    let dir = TempDir::new().unwrap();
    let owner = Owner::current().unwrap();
    let mut state = State {
        job: Some(owner.clone()),
        ..State::default()
    };
    state.batches.insert(
        "cycle-1".to_owned(),
        batch(&[7], "abc", BatchStatus::Running),
    );
    common::seed(dir.path(), state);
    let api = FakeApi::new().head(7, "abc");

    let request = wonton_onboard::finish::FinishRequest {
        recover: false,
        prs: [7].into(),
    };
    let outcome = finish(dir.path(), &request, &api, Duration::from_millis(50)).unwrap();
    let Outcome::ConflictAborted(reasons) = outcome else {
        panic!("应当是活 owner 冲突");
    };
    assert!(
        reasons.iter().all(|reason| reason.contains("alive")),
        "{reasons:?}"
    );
    assert!(dir.path().join(".git/wonton-finish.json").exists());

    let board = Board::open_existing(dir.path()).unwrap();
    assert_eq!(board.state().job, Some(owner));

    assert_eq!(
        finish(dir.path(), &request, &api, Duration::from_millis(50)).unwrap(),
        Outcome::JsonPending
    );
}
