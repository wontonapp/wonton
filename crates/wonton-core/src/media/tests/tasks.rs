use super::*;

#[test]
fn a_created_task_reads_back_with_every_field() {
    let (_dir, store) = store();
    let task = upload_task(1, ALICE);
    store.create_task(&task).unwrap();

    let back = store.task(ALICE, task.id).unwrap().expect("刚建的任务");
    assert_eq!(back, task);

    let offsets = store.item_offsets(ALICE, task.id).unwrap();
    assert_eq!(
        offsets,
        vec![("one".to_string(), 7), ("two".to_string(), 0)]
    );
}

#[test]
fn items_can_be_appended_and_saved_by_offset() {
    let (_dir, store) = store();
    let task = upload_task(1, ALICE);
    store.create_task(&task).unwrap();
    store
        .enqueue_items(ALICE, task.id, &[item("three", 0)])
        .unwrap();

    let mut saved = item("one", 4096);
    saved.state = ItemState::Running;
    assert!(store.save_item(ALICE, task.id, &saved).unwrap());

    let back = store.task(ALICE, task.id).unwrap().expect("任务还在");
    let TaskTarget::Upload(items) = &back.target else {
        panic!("上传任务的目标应该是上传");
    };
    assert_eq!(items.len(), 3);
    assert_eq!(items[0].upload_offset, 4096);
    assert_eq!(items[0].state, ItemState::Running);

    assert!(!store.save_item(BOB, task.id, &item("one", 1)).unwrap());
    assert!(!store.save_item(ALICE, task.id, &item("ghost", 1)).unwrap());
}

#[test]
fn tasks_can_be_filtered_by_kind_and_state() {
    let (_dir, store) = store();
    let mut queued = upload_task(1, ALICE);
    queued.state = TaskState::Queued;
    let mut download = upload_task(2, ALICE);
    download.kind = TaskKind::Download;
    download.target = TaskTarget::Download(media_item("movie"));
    download.state = TaskState::Running;
    store.create_task(&queued).unwrap();
    store.create_task(&download).unwrap();

    let uploads = store
        .tasks(Some(ALICE), Some(&TaskKind::Upload), None)
        .unwrap();
    assert_eq!(uploads.len(), 1);
    assert_eq!(uploads[0].id, queued.id);

    let running = store
        .tasks(Some(ALICE), None, Some(&TaskState::Running))
        .unwrap();
    assert_eq!(running.len(), 1);
    assert_eq!(running[0].id, download.id);
    assert_eq!(
        store
            .tasks(
                Some(ALICE),
                Some(&TaskKind::Download),
                Some(&TaskState::Queued)
            )
            .unwrap()
            .len(),
        0
    );
}

#[test]
fn save_task_leaves_items_to_save_item() {
    let (_dir, store) = store();
    let task = upload_task(1, ALICE);
    store.create_task(&task).unwrap();

    let mut changed = store.task(ALICE, task.id).unwrap().unwrap();
    let expected = changed.state.clone();
    changed.progress.done = 2;
    let TaskTarget::Upload(items) = &mut changed.target else {
        panic!("上传任务的目标应该是上传");
    };
    items[0].upload_offset = 4096;
    items[0].state = ItemState::Running;
    assert!(
        store
            .save_task(&changed, &expected, changed.retry_count)
            .unwrap()
    );

    let stored = store.task(ALICE, task.id).unwrap().unwrap();
    assert_eq!(stored.progress.done, 2);
    let TaskTarget::Upload(items) = stored.target else {
        panic!("上传任务的目标应该是上传");
    };
    assert_eq!(items[0].upload_offset, 7);
    assert_eq!(items[0].state, ItemState::Queued);

    let mut offset_only = items[0].clone();
    offset_only.upload_offset = 4096;
    store.save_item(ALICE, task.id, &offset_only).unwrap();
    assert_eq!(store.item_offsets(ALICE, task.id).unwrap()[0].1, 4096);
}

#[test]
fn save_task_only_lands_on_the_state_and_count_that_were_read() {
    let (_dir, store) = store();
    let task = upload_task(1, ALICE);
    store.create_task(&task).unwrap();

    // 别处先取消
    let mut cancelled = task.clone();
    cancelled.state = TaskState::Cancelled;
    assert!(
        store
            .save_task(&cancelled, &task.state, task.retry_count)
            .unwrap()
    );

    // 后到的等待网络拿旧状态写，落不上，取消不被覆盖
    let mut stale = task.clone();
    stale.state = TaskState::WaitingNetwork;
    assert!(
        !store
            .save_task(
                &stale,
                &TaskState::Paused(PauseReason::LowBattery),
                task.retry_count
            )
            .unwrap()
    );
    assert_eq!(
        store.task(ALICE, stale.id).unwrap().unwrap().state,
        TaskState::Cancelled
    );
}

#[test]
fn save_task_only_lands_on_a_retry_count_that_was_read() {
    let (_dir, store) = store();
    let task = upload_task(1, ALICE);
    store.create_task(&task).unwrap();

    let read = task.retry_count;

    // 一次失败先加了计数
    let mut first = task.clone();
    first.retry_count += 1;
    assert!(store.save_task(&first, &first.state, read).unwrap());

    // 另一次还拿老计数写，状态一样也落不上
    let mut second = task;
    second.retry_count += 1;
    assert!(!store.save_task(&second, &second.state, read).unwrap());
    assert_eq!(
        store.task(ALICE, second.id).unwrap().unwrap().retry_count,
        3
    );
}

#[test]
fn enqueueing_on_a_missing_or_download_task_errors() {
    let (_dir, store) = store();
    let missing = TaskId::from_u128(9);
    let err = store
        .enqueue_items(ALICE, missing, &[item("x", 0)])
        .unwrap_err();
    assert!(matches!(err, MediaStoreError::TaskNotFound { .. }), "{err}");

    let mut download = upload_task(1, ALICE);
    download.kind = TaskKind::Download;
    download.target = TaskTarget::Download(media_item("movie"));
    store.create_task(&download).unwrap();
    let err = store
        .enqueue_items(ALICE, download.id, &[item("x", 0)])
        .unwrap_err();
    assert!(
        matches!(err, MediaStoreError::TaskNotUpload { .. }),
        "{err}"
    );
}
