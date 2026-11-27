use super::*;

#[test]
fn a_created_session_reads_back_with_every_field() {
    let (_dir, store) = store();
    let session = session(ALICE, 2);
    store.create_session(&session).unwrap();
    assert_eq!(store.stored_session(session.id).unwrap().session, session);
    assert_eq!(store.sessions(Some(ALICE)).unwrap(), vec![session]);
}

#[test]
fn only_one_session_per_person_can_be_active() {
    let (_dir, store) = store();
    let first = session(ALICE, 1);
    store.create_session(&first).unwrap();

    let err = store.create_session(&session(ALICE, 2)).unwrap_err();
    assert!(
        matches!(err, MediaStoreError::ActiveSessionExists { .. }),
        "{err}"
    );
    assert_eq!(
        store.active_session(ALICE).unwrap().unwrap().session.id,
        first.id
    );

    let tracked = store.stored_session(first.id).unwrap();
    let mut stopped = first.clone();
    stopped.state = TaskState::Cancelled;
    stopped.stop_reason = Some(StopReason::TakenOver);
    assert!(store.save_session_cas(&stopped, tracked.version).unwrap());
    assert!(store.active_session(ALICE).unwrap().is_none());

    store.create_session(&session(ALICE, 2)).unwrap();
    assert_eq!(
        store.active_session(ALICE).unwrap().unwrap().session.id,
        TaskId::from_u128(2)
    );
    let old = store.stored_session(first.id).unwrap().session;
    assert_eq!(old.stop_reason, Some(StopReason::TakenOver));
    assert_eq!(old.position_ms, 42_000);
}

#[test]
fn a_session_write_needs_the_version_that_was_read() {
    let (_dir, store) = store();
    let session = session(ALICE, 1);
    store.create_session(&session).unwrap();
    let mut stale = store.stored_session(session.id).unwrap();

    let mut first = stale.session.clone();
    first.position_ms = 1;
    assert!(store.save_session_cas(&first, stale.version).unwrap());

    stale.session.position_ms = 2;
    assert!(
        !store
            .save_session_cas(&stale.session, stale.version)
            .unwrap()
    );
    assert_eq!(
        store
            .stored_session(session.id)
            .unwrap()
            .session
            .position_ms,
        1
    );
    assert!(!store.save_session_cas(&stale.session, 99).unwrap());
}

#[test]
fn a_missing_session_reports_its_id() {
    let (_dir, store) = store();
    let ghost = TaskId::from_u128(7);
    let err = store.stored_session(ghost).unwrap_err();
    assert!(
        matches!(err, MediaStoreError::SessionNotFound { session, .. } if session == ghost),
        "{err}"
    );
}
