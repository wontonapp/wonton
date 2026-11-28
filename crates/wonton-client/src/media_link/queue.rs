//! 队列改动（ADR-0113 第 4 节）：带读到的版本发给核心。别处先改了（409，体是当前队列）就换上当前队列，
//! 把这次改动按“同一项”挪到新的一版上重放一次：删的、跳去的、插在谁前面，都按那一项的编号重新找位置。
//! 找不准（本机没见过那一版）不猜，如实报 `stale_revision`，请用户看过再改。整条替换、清空、换模式
//! 不看位置，原样再发一次。

use wonton_protocol::media::api::QueueChange;
use wonton_protocol::media::{MediaFailure, MediaFailureCode, Queue, QueueOp, Revision};

use super::api::{MediaApi, QueueReply, failure};

/// 发一次改动；拿到的每一版队列（改成的、别处先改的）都交给 `adopt`，视图跟着换。
pub(super) async fn change(
    api: &MediaApi,
    before: Option<&Queue>,
    revision: Revision,
    op: QueueOp,
    adopt: impl Fn(Queue),
) -> Result<(), MediaFailure> {
    let mut seen = before.filter(|queue| queue.revision == revision).cloned();
    let mut change = QueueChange { revision, op };
    for _ in 0..2 {
        let current = match api.change_queue(&change).await? {
            QueueReply::Applied(queue) => {
                adopt(queue);
                return Ok(());
            }
            QueueReply::Stale(queue) => queue,
        };
        let replay = rebase(&change.op, seen.as_ref(), &current);
        adopt(current.clone());
        match replay? {
            Some(op) => {
                change = QueueChange {
                    revision: current.revision,
                    op,
                }
            }
            // 别处已经做了同一件事（要删的那项已经不在了）。
            None => return Ok(()),
        }
        seen = Some(current);
    }
    Err(stale(
        "the queue kept changing elsewhere; this change was not applied",
    ))
}

fn stale(message: &str) -> MediaFailure {
    failure(MediaFailureCode::StaleRevision, message)
}

/// 把读到 `seen` 时发的改动挪到 `now` 上；`None` 是不用再发了。
fn rebase(
    op: &QueueOp,
    seen: Option<&Queue>,
    now: &Queue,
) -> Result<Option<QueueOp>, MediaFailure> {
    let unknown =
        || stale("the queue changed elsewhere first; this positional change no longer fits");
    // 读到的那一版里第 `index` 项，在新的一版里排第几（`None` 是已经不在了）。同一项在队列里出现几次时按
    // 第几次出现对上；出现的次数变了就分不清是哪一个，外面的 `None` 当对不上。
    let moved = |index: usize| -> Option<Option<usize>> {
        let seen = seen?;
        let id = &seen.entries.get(index)?.id;
        let places = |queue: &Queue| -> Vec<usize> {
            (0..queue.entries.len())
                .filter(|at| &queue.entries[*at].id == id)
                .collect()
        };
        let (before, after) = (places(seen), places(now));
        if after.is_empty() {
            return Some(None);
        }
        if after.len() != before.len() {
            return None;
        }
        let nth = before.iter().position(|at| *at == index)?;
        Some(Some(after[nth]))
    };
    match op {
        QueueOp::Read
        | QueueOp::Clear
        | QueueOp::SetMode(_)
        | QueueOp::Replace { .. }
        | QueueOp::Enqueue { at: None, .. } => Ok(Some(op.clone())),
        QueueOp::Enqueue { at: Some(at), item } => {
            let at = if seen.is_some_and(|seen| *at >= seen.entries.len()) {
                None
            } else {
                // 原来那项没了就接在末尾。
                moved(*at).ok_or_else(unknown)?
            };
            Ok(Some(QueueOp::Enqueue {
                at,
                item: item.clone(),
            }))
        }
        QueueOp::Remove { index } => Ok(moved(*index)
            .ok_or_else(unknown)?
            .map(|index| QueueOp::Remove { index })),
        QueueOp::Jump { index } => match moved(*index).ok_or_else(unknown)? {
            Some(index) => Ok(Some(QueueOp::Jump { index })),
            None => Err(failure(
                MediaFailureCode::MediaGone,
                "the item to jump to was removed from the queue elsewhere",
            )),
        },
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use axum::Json;
    use axum::extract::State;
    use axum::http::StatusCode;
    use axum::routing::post;
    use url::Url;
    use wonton_protocol::media::api::{QueueChange, queue_path};
    use wonton_protocol::media::{
        MediaFailureCode, MediaItemRef, MediaKind, Queue, QueueMode, QueueOp,
    };

    use super::change;
    use crate::media_link::api::MediaApi;

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

    fn queue(revision: u64, ids: &[&str]) -> Queue {
        Queue {
            revision,
            entries: ids.iter().map(|id| item(id)).collect(),
            cursor: None,
            mode: QueueMode::Sequential,
            sleep_at_ms: None,
        }
    }

    struct Core {
        replies: Mutex<Vec<(StatusCode, Queue)>>,
        seen: Mutex<Vec<QueueChange>>,
    }

    /// 假核心：按顺序回 `replies`，记下每次收到的改动。
    async fn fake_core(replies: Vec<(StatusCode, Queue)>) -> (MediaApi, Arc<Core>) {
        let core = Arc::new(Core {
            replies: Mutex::new(replies),
            seen: Mutex::new(Vec::new()),
        });
        let router = axum::Router::new()
            .route(
                &queue_path(),
                post(
                    |State(core): State<Arc<Core>>, Json(change): Json<QueueChange>| async move {
                        core.seen.lock().unwrap().push(change);
                        let (status, queue) = core.replies.lock().unwrap().remove(0);
                        (status, Json(queue))
                    },
                ),
            )
            .with_state(core.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = Url::parse(&format!("http://{}", listener.local_addr().unwrap())).unwrap();
        tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        (
            MediaApi::new(base, reqwest::Client::new(), "phone".into()),
            core,
        )
    }

    async fn run(
        api: &MediaApi,
        before: Option<&Queue>,
        op: QueueOp,
    ) -> (Result<(), wonton_protocol::media::MediaFailure>, Vec<u64>) {
        let adopted = Mutex::new(Vec::new());
        let revision = before.map_or(1, |queue| queue.revision);
        let result = change(api, before, revision, op, |queue| {
            adopted.lock().unwrap().push(queue.revision);
        })
        .await;
        (result, adopted.into_inner().unwrap())
    }

    #[tokio::test]
    async fn the_same_item_queued_twice_is_told_apart_by_which_one_it_was() {
        // 删第二个 a：别处在前面插了一项，删的还是第二个 a。
        let before = queue(1, &["a", "b", "a"]);
        let (api, core) = fake_core(vec![
            (StatusCode::CONFLICT, queue(2, &["x", "a", "b", "a"])),
            (StatusCode::OK, queue(3, &["x", "a", "b"])),
        ])
        .await;
        run(&api, Some(&before), QueueOp::Remove { index: 2 })
            .await
            .0
            .unwrap();
        assert_eq!(
            core.seen.lock().unwrap()[1].op,
            QueueOp::Remove { index: 3 }
        );
        // 别处已经删掉了一个 a：剩下那个是不是要删的这个分不清，报出来，不猜。
        let (api, _) = fake_core(vec![(StatusCode::CONFLICT, queue(2, &["b", "a"]))]).await;
        let (result, _) = run(&api, Some(&before), QueueOp::Remove { index: 2 }).await;
        assert_eq!(result.unwrap_err().code, MediaFailureCode::StaleRevision);
    }

    #[tokio::test]
    async fn a_stale_change_is_replayed_on_the_same_item() {
        let before = queue(1, &["a", "b"]);
        let (api, core) = fake_core(vec![
            (StatusCode::CONFLICT, queue(2, &["x", "a", "b"])),
            (StatusCode::OK, queue(3, &["x", "a"])),
        ])
        .await;
        let (result, adopted) = run(&api, Some(&before), QueueOp::Remove { index: 1 }).await;
        result.unwrap();
        assert_eq!(adopted, [2, 3]);
        let seen = core.seen.lock().unwrap();
        assert_eq!(
            seen.iter()
                .map(|c| (c.revision, c.op.clone()))
                .collect::<Vec<_>>(),
            [
                (1, QueueOp::Remove { index: 1 }),
                (2, QueueOp::Remove { index: 2 })
            ]
        );
    }

    #[tokio::test]
    async fn a_change_that_no_longer_fits_is_reported_not_guessed() {
        // 要跳去的那项被别处删了：换上当前队列，报那项没了，不跳到别的上。
        let before = queue(1, &["a", "b"]);
        let (api, core) = fake_core(vec![(StatusCode::CONFLICT, queue(2, &["a"]))]).await;
        let (result, adopted) = run(&api, Some(&before), QueueOp::Jump { index: 1 }).await;
        assert_eq!(result.unwrap_err().code, MediaFailureCode::MediaGone);
        assert_eq!(adopted, [2]);
        // 本机没见过发改动时的那一版：按位置的改动找不准，如实报。
        let (api, _) = fake_core(vec![(StatusCode::CONFLICT, queue(5, &["a", "b"]))]).await;
        let (result, adopted) = run(&api, None, QueueOp::Remove { index: 0 }).await;
        assert_eq!(result.unwrap_err().code, MediaFailureCode::StaleRevision);
        assert_eq!(adopted, [5]);
        assert_eq!(core.seen.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn a_stale_replace_is_sent_again_as_it_was() {
        let (api, core) = fake_core(vec![
            (StatusCode::CONFLICT, queue(2, &["x"])),
            (StatusCode::OK, queue(3, &["a", "b"])),
        ])
        .await;
        let replace = QueueOp::Replace {
            entries: vec![item("a"), item("b")],
            cursor: Some(1),
        };
        let (result, adopted) = run(&api, None, replace.clone()).await;
        result.unwrap();
        assert_eq!(adopted, [2, 3]);
        let seen = core.seen.lock().unwrap();
        assert_eq!(
            seen.iter()
                .map(|c| (c.revision, c.op.clone()))
                .collect::<Vec<_>>(),
            [(1, replace.clone()), (2, replace)]
        );
    }

    #[tokio::test]
    async fn a_removal_someone_else_already_made_needs_nothing_more() {
        let before = queue(1, &["a", "b"]);
        let (api, core) = fake_core(vec![(StatusCode::CONFLICT, queue(2, &["a"]))]).await;
        let (result, _) = run(&api, Some(&before), QueueOp::Remove { index: 1 }).await;
        result.unwrap();
        assert_eq!(core.seen.lock().unwrap().len(), 1);
    }
}
