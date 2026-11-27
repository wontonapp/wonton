//! 队列的改动（ADR-0113 第 4 节，M10）：每次改动带客户端读到的版本，版本不等就不改、回当前队列，
//! 客户端按它重放（接口上是 409）。版本比对在库里做（[`Store::save_queue_cas`]），两台设备同时改
//! 只有一台落库。
//!
//! 只改队列本身；换到哪一首由客户端接着起播，`Next`/`Prev` 走会话控制（[`SessionService::control`]）。

use wonton_protocol::media::{Queue, QueueMode, QueueOp, Revision};

use super::{SessionService, SessionServiceError};

/// 一次改动的结果。
#[derive(Debug, Clone, PartialEq)]
pub enum QueueChanged {
    Applied(Queue),
    /// 版本对不上，没改；值是当前队列。
    Stale(Queue),
}

/// 还没有队列的人：第 0 版、空的。
fn empty() -> Queue {
    Queue {
        revision: 0,
        entries: Vec::new(),
        cursor: None,
        mode: QueueMode::Sequential,
        sleep_at_ms: None,
    }
}

impl SessionService {
    pub fn queue(&self, person: &str) -> Result<Queue, SessionServiceError> {
        Ok(self.store.queue(person)?.unwrap_or_else(empty))
    }

    /// `Read` 不看版本，原样回当前队列。
    pub fn change_queue(
        &self,
        person: &str,
        revision: Revision,
        op: QueueOp,
    ) -> Result<QueueChanged, SessionServiceError> {
        let current = self.queue(person)?;
        if matches!(op, QueueOp::Read) {
            return Ok(QueueChanged::Applied(current));
        }
        if current.revision != revision {
            return Ok(QueueChanged::Stale(current));
        }
        let mut next = current;
        apply(&mut next, op)?;
        next.bump();
        if self.store.save_queue_cas(person, &next, revision)? {
            Ok(QueueChanged::Applied(next))
        } else {
            Ok(QueueChanged::Stale(self.queue(person)?))
        }
    }
}

fn apply(queue: &mut Queue, op: QueueOp) -> Result<(), SessionServiceError> {
    let len = queue.entries.len();
    let check = |index: usize| {
        if index < len {
            Ok(())
        } else {
            Err(SessionServiceError::QueueIndex { index, len })
        }
    };
    match op {
        QueueOp::Read => {}
        QueueOp::Enqueue { at, item } => {
            let at = at.unwrap_or(len).min(len);
            queue.entries.insert(at, item);
            queue.cursor = match queue.cursor {
                Some(cursor) if at <= cursor => Some(cursor + 1),
                None if len == 0 => Some(0),
                cursor => cursor,
            };
        }
        QueueOp::Remove { index } => {
            check(index)?;
            queue.entries.remove(index);
            queue.cursor = match queue.cursor {
                _ if queue.entries.is_empty() => None,
                Some(cursor) if index < cursor => Some(cursor - 1),
                Some(cursor) => Some(cursor.min(queue.entries.len() - 1)),
                None => None,
            };
        }
        QueueOp::Jump { index } => {
            check(index)?;
            queue.cursor = Some(index);
        }
        QueueOp::Clear => {
            queue.entries.clear();
            queue.cursor = None;
        }
        QueueOp::SetMode(mode) => queue.mode = mode,
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use wonton_protocol::media::{MediaItemRef, MediaKind};

    use super::*;
    use crate::media::Store;

    const ALICE: &str = "@alice:example.com";

    fn item(id: &str) -> MediaItemRef {
        MediaItemRef {
            id: id.into(),
            kind: MediaKind::Audio,
            title: id.into(),
            artist: None,
            album: None,
            artwork_url: None,
            duration_ms: None,
            series: None,
            index: None,
            played: false,
            starred: false,
            source: None,
        }
    }

    fn service() -> (tempfile::TempDir, SessionService) {
        let dir = tempfile::TempDir::new().unwrap();
        let store = Arc::new(Store::open(dir.path()).unwrap());
        (dir, SessionService::new(store))
    }

    fn applied(change: Result<QueueChanged, SessionServiceError>) -> Queue {
        match change.unwrap() {
            QueueChanged::Applied(queue) => queue,
            QueueChanged::Stale(queue) => panic!("没落库，当前是第 {} 版", queue.revision),
        }
    }

    fn ids(queue: &Queue) -> Vec<&str> {
        queue.entries.iter().map(|e| e.id.as_str()).collect()
    }

    #[test]
    fn a_change_from_an_old_revision_is_refused_with_the_current_queue() {
        let (_dir, sessions) = service();
        let enqueue = |at, id| QueueOp::Enqueue { at, item: item(id) };
        let first = applied(sessions.change_queue(ALICE, 0, enqueue(None, "a")));
        assert_eq!((first.revision, first.cursor), (1, Some(0)));
        // 另一台设备还拿着第 0 版：不改，回第 1 版让它重放。
        match sessions.change_queue(ALICE, 0, enqueue(None, "b")).unwrap() {
            QueueChanged::Stale(current) => assert_eq!(current, first),
            other => panic!("旧版本落库了：{other:?}"),
        }
        let read = applied(sessions.change_queue(ALICE, 0, QueueOp::Read));
        assert_eq!(read, first);
    }

    #[test]
    fn the_cursor_stays_on_the_same_track_as_the_queue_changes() {
        let (_dir, sessions) = service();
        let mut queue = sessions.queue(ALICE).unwrap();
        for op in [
            QueueOp::Enqueue {
                at: None,
                item: item("a"),
            },
            QueueOp::Enqueue {
                at: None,
                item: item("b"),
            },
            QueueOp::Jump { index: 1 },
            QueueOp::Enqueue {
                at: Some(0),
                item: item("c"),
            },
        ] {
            queue = applied(sessions.change_queue(ALICE, queue.revision, op));
        }
        assert_eq!((ids(&queue), queue.cursor), (vec!["c", "a", "b"], Some(2)));
        queue = applied(sessions.change_queue(ALICE, queue.revision, QueueOp::Remove { index: 0 }));
        assert_eq!((ids(&queue), queue.cursor), (vec!["a", "b"], Some(1)));
        // 删掉正在放的最后一首，指针落到新的最后一首；删空了就没有指针。
        queue = applied(sessions.change_queue(ALICE, queue.revision, QueueOp::Remove { index: 1 }));
        assert_eq!((ids(&queue), queue.cursor), (vec!["a"], Some(0)));
        queue = applied(sessions.change_queue(ALICE, queue.revision, QueueOp::Clear));
        assert_eq!((queue.entries.len(), queue.cursor), (0, None));
    }

    #[test]
    fn an_index_past_the_end_is_refused_and_changes_nothing() {
        let (_dir, sessions) = service();
        let queue = applied(sessions.change_queue(
            ALICE,
            0,
            QueueOp::Enqueue {
                at: None,
                item: item("a"),
            },
        ));
        let error = sessions
            .change_queue(ALICE, queue.revision, QueueOp::Jump { index: 3 })
            .unwrap_err();
        assert!(matches!(
            error,
            SessionServiceError::QueueIndex { index: 3, len: 1 }
        ));
        assert_eq!(sessions.queue(ALICE).unwrap(), queue);
    }
}
