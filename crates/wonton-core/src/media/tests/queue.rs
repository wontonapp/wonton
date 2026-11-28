use super::*;

#[test]
fn a_saved_queue_reads_back() {
    let (_dir, store) = store();
    let mut queue = queue();
    store.save_queue(ALICE, &queue).unwrap();
    assert_eq!(store.queue(ALICE).unwrap(), Some(queue.clone()));

    queue.bump();
    queue.cursor = None;
    store.save_queue(ALICE, &queue).unwrap();
    assert_eq!(store.queue(ALICE).unwrap(), Some(queue));
}
