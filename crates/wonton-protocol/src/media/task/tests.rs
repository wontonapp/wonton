//! 契约测试：状态机转移、退避、serde 往返和未知取值落到 `Other`。

use super::*;
use uuid::Uuid;

fn failure(code: MediaFailureCode) -> MediaFailure {
    MediaFailure {
        code,
        message: "炸了".into(),
        retry_after_secs: None,
    }
}

fn queued() -> TaskState {
    TaskState::Queued
}

fn preparing() -> TaskState {
    TaskState::Preparing
}

fn running() -> TaskState {
    TaskState::Running
}

fn paused() -> TaskState {
    TaskState::Paused(PauseReason::User)
}

fn waiting_network() -> TaskState {
    TaskState::WaitingNetwork
}

fn waiting_power() -> TaskState {
    TaskState::WaitingPower
}

#[test]
fn legal_transitions_pass() {
    let legal = [
        (queued(), preparing()),
        (queued(), TaskState::Cancelled),
        (queued(), paused()),
        (queued(), waiting_network()),
        (queued(), waiting_power()),
        (preparing(), running()),
        (
            preparing(),
            TaskState::Failed(failure(MediaFailureCode::Protocol)),
        ),
        (preparing(), TaskState::Cancelled),
        (running(), paused()),
        (running(), waiting_network()),
        (running(), waiting_power()),
        (running(), TaskState::Done),
        (running(), TaskState::Partial),
        (
            running(),
            TaskState::Failed(failure(MediaFailureCode::UpstreamError)),
        ),
        (running(), TaskState::Cancelled),
        (preparing(), TaskState::Done),
        (preparing(), TaskState::Partial),
        (paused(), running()),
        (paused(), TaskState::Paused(PauseReason::Agent)),
        (paused(), TaskState::Cancelled),
        (paused(), queued()),
        (waiting_network(), running()),
        (waiting_network(), paused()),
        (waiting_network(), queued()),
        (waiting_network(), waiting_network()),
        (
            waiting_network(),
            TaskState::Failed(failure(MediaFailureCode::TransportUnavailable)),
        ),
        (waiting_network(), TaskState::Cancelled),
        (waiting_power(), running()),
        (waiting_power(), queued()),
        (waiting_power(), waiting_power()),
        (waiting_power(), paused()),
        (waiting_power(), TaskState::Cancelled),
    ];
    for (from, to) in legal {
        assert_eq!(
            from.clone().transition(to.clone()),
            Ok(to.clone()),
            "{from:?}"
        );
        assert_eq!(TaskState::try_from((from, to.clone())), Ok(to));
    }
}

#[test]
fn terminal_states_never_go_back_to_running() {
    let terminals = [
        TaskState::Done,
        TaskState::Partial,
        TaskState::Failed(failure(MediaFailureCode::MediaGone)),
        TaskState::Cancelled,
    ];
    let targets = [
        queued(),
        preparing(),
        running(),
        paused(),
        waiting_network(),
        waiting_power(),
    ];
    for terminal in terminals {
        for target in &targets {
            assert!(
                terminal.clone().transition(target.clone()).is_err(),
                "{terminal:?} → {target:?}"
            );
        }
    }
}

#[test]
fn illegal_transition_names_both_states() {
    let error = TaskState::Done.transition(TaskState::Running).unwrap_err();
    assert_eq!(error.from, TaskState::Done);
    assert_eq!(error.to, TaskState::Running);
    assert_eq!(error.to_string(), "任务状态不能从 done 变到 running");
}

#[test]
fn every_failure_code_has_its_backoff() {
    let expected = [
        (MediaFailureCode::LocalSourceUnavailable, Backoff::None),
        (MediaFailureCode::LocalPermissionDenied, Backoff::None),
        (MediaFailureCode::LocalStorageFull, Backoff::None),
        (MediaFailureCode::LocalIo, Backoff::Once),
        (MediaFailureCode::TransportUnavailable, Backoff::Standard),
        (MediaFailureCode::Protocol, Backoff::Standard),
        (MediaFailureCode::AuthenticationRequired, Backoff::None),
        (MediaFailureCode::PermissionDenied, Backoff::None),
        (MediaFailureCode::UnsupportedByDevice, Backoff::None),
        (MediaFailureCode::RateLimited, Backoff::RetryAfter),
        (MediaFailureCode::MediaGone, Backoff::None),
        (MediaFailureCode::PluginUnavailable, Backoff::ThreeTimes),
        (MediaFailureCode::UpstreamError, Backoff::Standard),
        (MediaFailureCode::ServerStorageFull, Backoff::None),
        (MediaFailureCode::SuspendedBySystem, Backoff::SystemPause),
        (MediaFailureCode::SourceRequired, Backoff::None),
        (MediaFailureCode::Other("quantum".into()), Backoff::None),
    ];
    assert_eq!(expected.len(), 17);
    for (code, backoff) in expected {
        assert_eq!(code.backoff(), backoff, "{code:?}");
    }
}

#[test]
fn failures_without_retry_after_omit_the_field() {
    let json = serde_json::to_value(failure(MediaFailureCode::RateLimited)).unwrap();
    assert_eq!(json["code"], "rate_limited");
    assert!(json.get("retry_after_secs").is_none());

    let limited = MediaFailure {
        retry_after_secs: Some(30),
        ..failure(MediaFailureCode::RateLimited)
    };
    let json = serde_json::to_value(&limited).unwrap();
    assert_eq!(json["retry_after_secs"], 30);
}

#[test]
fn task_and_targets_roundtrip() {
    let task = MediaTask {
        id: Uuid::nil(),
        kind: TaskKind::Upload,
        person: "Alice".into(),
        device: "phone".into(),
        target: TaskTarget::Upload(vec![UploadItem {
            item_id: "i1".into(),
            path: "DCIM/a.jpg".into(),
            size: 2048,
            checksum: Some("sha256:x".into()),
            taken_at: Some(1_700_000_000_000),
            state: ItemState::Failed(failure(MediaFailureCode::LocalIo)),
            upload_offset: 512,
        }]),
        state: TaskState::Partial,
        progress: Progress {
            done: 1,
            total: Some(2),
            unit: ProgressUnit::Items,
        },
        retry_count: 2,
        retry_of: Some(Uuid::nil()),
        next_attempt_at: Some(1_700_000_010),
        created_at: 1_700_000_000_000,
        partial_failures: vec![FailedItem {
            item_id: "i1".into(),
            path: "DCIM/a.jpg".into(),
            failure: failure(MediaFailureCode::LocalIo),
        }],
    };
    let json = serde_json::to_value(&task).unwrap();
    let back: MediaTask = serde_json::from_value(json.clone()).unwrap();
    assert_eq!(serde_json::to_value(&back).unwrap(), json);

    let download = TaskTarget::Download(super::super::tests::item());
    let json = serde_json::to_value(&download).unwrap();
    assert_eq!(json["kind"], "download");
    let back: TaskTarget = serde_json::from_value(json.clone()).unwrap();
    assert_eq!(serde_json::to_value(&back).unwrap(), json);
}

#[test]
fn item_state_failure_inlines_the_tag() {
    let json =
        serde_json::to_value(ItemState::Failed(failure(MediaFailureCode::MediaGone))).unwrap();
    assert_eq!(json["state"], "failed");
    assert_eq!(json["code"], "media_gone");
    let back: ItemState = serde_json::from_value(json.clone()).unwrap();
    assert_eq!(serde_json::to_value(&back).unwrap(), json);
}

#[test]
fn unknown_task_target_and_progress_unit_fall_to_other() {
    let mut json = serde_json::to_value(MediaTask {
        id: Uuid::nil(),
        kind: TaskKind::Upload,
        person: "Alice".into(),
        device: "phone".into(),
        target: TaskTarget::Download(super::super::tests::item()),
        state: TaskState::Running,
        progress: Progress {
            done: 1,
            total: Some(2),
            unit: ProgressUnit::Items,
        },
        retry_count: 0,
        retry_of: None,
        next_attempt_at: None,
        created_at: 1_700_000_000_000,
        partial_failures: Vec::new(),
    })
    .unwrap();
    json["target"] = serde_json::json!({"kind": "stream"});
    json["progress"]["unit"] = serde_json::json!("chunks");

    let back: MediaTask = serde_json::from_value(json.clone()).unwrap();
    assert_eq!(back.target, TaskTarget::Other("stream".into()));
    assert_eq!(back.progress.unit, ProgressUnit::Other("chunks".into()));
    assert_eq!(serde_json::to_value(&back).unwrap(), json);
}

#[test]
fn unknown_task_values_fall_to_other() {
    let kind: TaskKind = serde_json::from_str("\"migrate\"").unwrap();
    assert_eq!(kind, TaskKind::Other("migrate".into()));
    assert_eq!(
        serde_json::to_value(&kind).unwrap(),
        serde_json::json!("migrate")
    );

    let code: MediaFailureCode = serde_json::from_str("\"quantum\"").unwrap();
    assert_eq!(code, MediaFailureCode::Other("quantum".into()));
    assert_eq!(
        serde_json::to_value(&code).unwrap(),
        serde_json::json!("quantum")
    );

    let state: TaskState = serde_json::from_str("\"melting\"").unwrap();
    assert_eq!(state, TaskState::Other("melting".into()));
    assert_eq!(state.as_str(), "melting");
    assert_eq!(
        serde_json::to_value(&state).unwrap(),
        serde_json::json!("melting")
    );

    let item: ItemState = serde_json::from_str("{\"state\": \"frozen\", \"since\": 3}").unwrap();
    assert_eq!(item, ItemState::Other("frozen".into()));
    assert_eq!(
        serde_json::to_value(&item).unwrap(),
        serde_json::json!({"state": "frozen"})
    );
}

#[test]
fn unknown_tagged_task_state_keeps_the_tag_name() {
    let state: TaskState = serde_json::from_str("{\"migrating\": {\"a\": 1}}").unwrap();
    assert_eq!(state, TaskState::Other("migrating".into()));
    assert_eq!(
        serde_json::to_value(&state).unwrap(),
        serde_json::json!("migrating")
    );
}

#[test]
fn unknown_states_only_enter_and_leave_as_themselves() {
    let migrating = TaskState::Other("migrating".into());
    let paused = TaskState::Other("paused".into());
    assert_eq!(
        TaskState::Running.transition(migrating.clone()),
        Ok(migrating.clone())
    );
    assert_eq!(
        TaskState::Preparing.transition(migrating.clone()),
        Ok(migrating.clone())
    );
    assert_eq!(migrating.clone().transition(paused.clone()), Ok(paused));
    assert!(TaskState::Done.transition(migrating.clone()).is_err());
    assert!(TaskState::Cancelled.transition(migrating.clone()).is_err());
    assert!(migrating.transition(TaskState::Running).is_err());
}

#[test]
fn out_of_order_known_transitions_are_errors() {
    assert!(TaskState::Queued.transition(TaskState::Running).is_err());
    assert!(TaskState::Running.transition(TaskState::Preparing).is_err());
}

#[test]
fn default_task_options_use_one_concurrency() {
    let options = TaskOptions::default();
    assert_eq!(options.concurrency, 1);
    assert!(!options.wifi_only);
    assert!(options.albums.is_empty());
}

#[test]
fn task_options_default_to_one_concurrency() {
    let options: TaskOptions = serde_json::from_value(serde_json::json!({
        "wifi_only": false,
        "charging_only": false,
        "max_bitrate": null,
        "albums": [],
        "force_now": false
    }))
    .unwrap();
    assert_eq!(options.concurrency, 1);
    assert!(!options.wifi_only);
    assert!(!options.charging_only);
    assert!(options.albums.is_empty());
    assert!(!options.force_now);
}
