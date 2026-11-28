//! 到这台设备的命令（ADR-0113 第 5 节）：连着家里时挂长轮询取命令，逐条执行再回执。
//!
//! 核心等回执最多 30 秒，没等到就给 Agent 报 `device_no_response`。这一版做不了的命令立即回失败回执，类别
//! `unsupported_by_device`，原文写明做不了的是什么：传输、备份、删离线副本归执行器（M18、M19），认不得的
//! 新命令也一样。不回执就是让核心白等 30 秒，再让 Agent 去猜是不是手机关机了。

use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Weak};
use std::time::Duration;

use tokio::runtime::Handle;

use reqwest::{Method, StatusCode};
use wonton_protocol::HomeStatus;
use wonton_protocol::media::api::{CommandAck, CommandList, command_ack_path, commands_path};
use wonton_protocol::media::{DeviceCommand, MediaAction, MediaFailure, MediaFailureCode};

use super::MediaLink;
use super::api::{MediaApi, failure, read, send};

/// 核心最长挂 25 秒，这边多等一会儿，免得每次都在它回之前先断。
const POLL_TIMEOUT: Duration = Duration::from_secs(40);
/// 取命令或回执失败后隔这么久再试；断开了就不再取。
const RETRY_AFTER: Duration = Duration::from_secs(5);

/// 同时只跑一条取命令的长轮询：断开又连上时，上一条可能还挂在请求上。
#[derive(Default)]
pub(super) struct Poller {
    running: Arc<AtomicBool>,
}

impl Poller {
    /// 没在跑就开一条；返回这次开没开。`run` 每次造一条新的轮询，断开时跑完。
    ///
    /// 跑完先放下标记，再看一眼 `connected`：轮询看见断开退出、到放下标记之间，可能刚好又连上了，那次
    /// `ensure` 看见标记还在就没开新的。不补这一下，连着却再也没人取命令。
    pub(super) fn ensure<R, Fut>(
        &self,
        runtime: &Handle,
        connected: impl Fn() -> bool + Send + 'static,
        run: R,
    ) -> bool
    where
        R: Fn() -> Fut + Send + 'static,
        Fut: Future<Output = ()> + Send,
    {
        if self.running.swap(true, Ordering::AcqRel) {
            return false;
        }
        let running = self.running.clone();
        runtime.spawn(async move {
            loop {
                run().await;
                running.store(false, Ordering::Release);
                if !connected() || running.swap(true, Ordering::AcqRel) {
                    break;
                }
            }
        });
        true
    }

    #[cfg(test)]
    fn running(&self) -> bool {
        self.running.load(Ordering::Acquire)
    }
}

/// 这条链接还在、连着家里。
pub(super) fn is_connected(link: &Weak<MediaLink>) -> bool {
    link.upgrade()
        .is_some_and(|link| link.home.view().status == HomeStatus::Connected)
}

/// 这条链接连着家里时的接口；断开或链接没了是 `None`。
pub(super) fn connected(link: &Weak<MediaLink>) -> Option<MediaApi> {
    if !is_connected(link) {
        return None;
    }
    link.upgrade()?.api().ok()
}

/// `connect` 给得出接口就一直取，给不出（断开了）就停。`after` 只记在内存里：重启或重开一条后从 0 取，核心
/// 回的是还没回执的；回执发出去前断开的那条会再收到一次。所以命令按 `command_id` 幂等执行，不能当成恰好一次。
pub(super) async fn follow<C, P, Fut>(connect: C, perform: P)
where
    C: Fn() -> Option<MediaApi>,
    P: Fn(MediaAction) -> Fut,
    Fut: Future<Output = Result<(), MediaFailure>>,
{
    let mut after = 0;
    while let Some(api) = connect() {
        let served = match poll(&api, after).await {
            Ok(commands) => serve(&api, commands, &perform).await,
            Err(failure) => Err(failure),
        };
        match served {
            Ok(last) => after = after.max(last),
            Err(failure) => {
                tracing::warn!(code = ?failure.code, error = %failure.message, "media commands: polling or acknowledging failed");
                tokio::time::sleep(RETRY_AFTER).await;
            }
        }
    }
}

/// 取一次：有命令就回，挂满没有回空。
pub(super) async fn poll(api: &MediaApi, after: u64) -> Result<Vec<DeviceCommand>, MediaFailure> {
    // 字段名照 [`CommandQuery`]：这边的 reqwest 没开 `query`，查询串自己拼。
    let query = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("device", api.device())
        .append_pair("after", &after.to_string())
        .finish();
    let request = api
        .request(Method::GET, &format!("{}?{query}", commands_path()))
        .timeout(POLL_TIMEOUT);
    let response = send(request).await?;
    if response.status() == StatusCode::NO_CONTENT {
        return Ok(Vec::new());
    }
    Ok(read::<CommandList>(response).await?.commands)
}

/// 按 `seq` 逐条执行并回执，返回回执到的最大 `seq`。回执发不出去就停在那一条，之后的下次再取。
pub(super) async fn serve<F, Fut>(
    api: &MediaApi,
    mut commands: Vec<DeviceCommand>,
    perform: &F,
) -> Result<u64, MediaFailure>
where
    F: Fn(MediaAction) -> Fut,
    Fut: Future<Output = Result<(), MediaFailure>>,
{
    commands.sort_by_key(|command| command.seq);
    let mut last = 0;
    for command in commands {
        let outcome = match cannot_run(&command.body) {
            Some(why) => Err(failure(MediaFailureCode::UnsupportedByDevice, why)),
            None => perform(command.body).await,
        };
        let ack = CommandAck {
            seq: command.seq,
            ok: outcome.is_ok(),
            failure: outcome.err(),
            task: None,
        };
        let request = api
            .request(Method::POST, &command_ack_path(command.command_id))
            .json(&ack);
        send(request).await?;
        last = command.seq;
    }
    Ok(last)
}

/// 这一版的客户端核做不了的命令和原因；`None` 是能做。
fn cannot_run(action: &MediaAction) -> Option<String> {
    let what = match action {
        MediaAction::Playback { .. }
        | MediaAction::Control { .. }
        | MediaAction::Stop { .. }
        | MediaAction::Queue { .. } => return None,
        MediaAction::StartUpload { .. } => "start an upload",
        MediaAction::StartDownload { .. } => "start a download",
        MediaAction::Backup { .. } => "run a backup",
        MediaAction::DeleteOffline { .. } => "delete offline copies",
        MediaAction::Other(kind) => {
            return Some(format!(
                "this client core does not know the command `{kind}`; update the app"
            ));
        }
    };
    Some(format!(
        "this client core cannot {what} yet; the transfer executor is not built"
    ))
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use axum::Json;
    use axum::extract::{Path, Query, State};
    use axum::http::StatusCode;
    use axum::response::IntoResponse;
    use axum::routing::{get, post};
    use url::Url;
    use wonton_protocol::media::api::{CommandAck, CommandList, CommandQuery, commands_path};
    use wonton_protocol::media::{
        DeviceCommand, MediaAction, MediaFailureCode, MediaKind, PlaybackSessionId, StopReason,
        TaskId, new_task_id,
    };

    use super::{Poller, follow, poll, serve};
    use crate::media_link::api::{MediaApi, failure};

    #[derive(Default)]
    struct Core {
        batches: Mutex<Vec<CommandList>>,
        queries: Mutex<Vec<CommandQuery>>,
        acks: Mutex<Vec<(TaskId, CommandAck)>>,
        /// 正挂着的取命令请求数，和见过的最大值。
        in_flight: AtomicUsize,
        most_in_flight: AtomicUsize,
    }

    async fn fake_core(batches: Vec<CommandList>) -> (MediaApi, Arc<Core>) {
        let core = Arc::new(Core {
            batches: Mutex::new(batches),
            ..Core::default()
        });
        let router = axum::Router::new()
            .route(
                &commands_path(),
                get(
                    |State(core): State<Arc<Core>>, Query(query): Query<CommandQuery>| async move {
                        core.queries.lock().unwrap().push(query);
                        let now = core.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
                        core.most_in_flight.fetch_max(now, Ordering::SeqCst);
                        // 挂一小会儿再回，两条轮询同时在跑时能被看见。
                        tokio::time::sleep(Duration::from_millis(5)).await;
                        core.in_flight.fetch_sub(1, Ordering::SeqCst);
                        let next = {
                            let mut batches = core.batches.lock().unwrap();
                            (!batches.is_empty()).then(|| batches.remove(0))
                        };
                        match next {
                            Some(batch) => Json(batch).into_response(),
                            None => StatusCode::NO_CONTENT.into_response(),
                        }
                    },
                ),
            )
            .route(
                "/_wonton/api/media/commands/{id}/ack",
                post(
                    |State(core): State<Arc<Core>>,
                     Path(id): Path<TaskId>,
                     Json(ack): Json<CommandAck>| async move {
                        core.acks.lock().unwrap().push((id, ack));
                        StatusCode::NO_CONTENT
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

    fn command(seq: u64, body: MediaAction) -> DeviceCommand {
        DeviceCommand {
            command_id: new_task_id(),
            seq,
            body,
        }
    }

    #[tokio::test]
    async fn commands_this_core_cannot_run_are_refused_at_once_instead_of_timing_out() {
        let stop = command(
            2,
            MediaAction::Stop {
                session: PlaybackSessionId::nil(),
                reason: StopReason::Agent,
            },
        );
        let batch = vec![
            command(3, MediaAction::Other("beam_me_up".into())),
            stop.clone(),
            command(
                1,
                MediaAction::Backup {
                    kinds: vec![MediaKind::Photo],
                },
            ),
        ];
        let (api, core) = fake_core(Vec::new()).await;
        let performed = Mutex::new(Vec::new());
        let last = serve(&api, batch, &|action| {
            performed.lock().unwrap().push(action);
            async { Ok(()) }
        })
        .await
        .unwrap();
        assert_eq!(last, 3);
        // 能做的照做；做不了的不碰执行，直接回失败回执。
        assert_eq!(*performed.lock().unwrap(), [stop.body]);
        let acks = core.acks.lock().unwrap();
        let seen: Vec<_> = acks
            .iter()
            .map(|(_, ack)| {
                (
                    ack.seq,
                    ack.ok,
                    ack.failure.as_ref().map(|f| f.code.clone()),
                )
            })
            .collect();
        assert_eq!(
            seen,
            [
                (1, false, Some(MediaFailureCode::UnsupportedByDevice)),
                (2, true, None),
                (3, false, Some(MediaFailureCode::UnsupportedByDevice)),
            ]
        );
        assert_eq!(acks[1].0, stop.command_id);
    }

    #[tokio::test]
    async fn a_command_that_failed_is_acked_with_its_failure() {
        let (api, core) = fake_core(Vec::new()).await;
        let control = command(
            7,
            MediaAction::Stop {
                session: PlaybackSessionId::nil(),
                reason: StopReason::User,
            },
        );
        serve(&api, vec![control], &|_| async {
            Err(failure(MediaFailureCode::MediaGone, "no such session"))
        })
        .await
        .unwrap();
        let acks = core.acks.lock().unwrap();
        let ack = &acks[0].1;
        assert!(!ack.ok);
        assert_eq!(
            ack.failure.as_ref().unwrap().code,
            MediaFailureCode::MediaGone
        );
    }

    #[tokio::test]
    async fn polling_names_this_device_and_what_it_already_acked() {
        let batch = CommandList {
            commands: vec![command(
                5,
                MediaAction::Playback {
                    session: PlaybackSessionId::nil(),
                },
            )],
        };
        let (api, core) = fake_core(vec![batch]).await;
        assert_eq!(poll(&api, 4).await.unwrap().len(), 1);
        // 挂满没有命令：核心回 204，这边当空。
        assert!(poll(&api, 5).await.unwrap().is_empty());
        let queries = core.queries.lock().unwrap();
        assert_eq!(
            queries
                .iter()
                .map(|q| (q.device.as_str(), q.after))
                .collect::<Vec<_>>(),
            [("phone", 4), ("phone", 5)]
        );
    }

    /// 等到条件成立，最多两秒。
    async fn until(what: &str, ready: impl Fn() -> bool) {
        for _ in 0..400 {
            if ready() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        panic!("两秒内没等到：{what}");
    }

    #[tokio::test]
    async fn polling_runs_while_connected_stops_on_disconnect_and_restarts_as_one() {
        let (api, core) = fake_core(Vec::new()).await;
        let connected = Arc::new(AtomicBool::new(true));
        let run = {
            let (api, connected) = (api.clone(), connected.clone());
            move || {
                let (api, connected) = (api.clone(), connected.clone());
                follow(
                    move || connected.load(Ordering::SeqCst).then(|| api.clone()),
                    |_| async { Ok(()) },
                )
            }
        };
        let polls = || core.queries.lock().unwrap().len();
        let runtime = tokio::runtime::Handle::current();
        let poller = Poller::default();
        let ensure = || {
            let connected = connected.clone();
            poller.ensure(
                &runtime,
                move || connected.load(Ordering::SeqCst),
                run.clone(),
            )
        };
        assert!(ensure());
        // 断开又连上时上一条还在：不开第二条。
        assert!(!ensure());
        until("连上后开始取", || polls() >= 3).await;
        connected.store(false, Ordering::SeqCst);
        until("断开后停下", || !poller.running()).await;
        let stopped_at = polls();
        tokio::time::sleep(Duration::from_millis(30)).await;
        assert_eq!(polls(), stopped_at, "断开后还在取");
        connected.store(true, Ordering::SeqCst);
        assert!(ensure());
        until("重连后接着取", || polls() > stopped_at + 2).await;
        assert_eq!(core.most_in_flight.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn a_reconnect_while_the_old_poll_is_exiting_still_gets_a_poll() {
        // 轮询看见断开退出了，放下标记时已经又连上：重连那次 ensure 没开新的，这里得接着取。
        let checks = Arc::new(AtomicUsize::new(0));
        let runs = Arc::new(AtomicUsize::new(0));
        let poller = Poller::default();
        let connected = {
            let checks = checks.clone();
            move || checks.fetch_add(1, Ordering::SeqCst) == 0
        };
        let run = {
            let runs = runs.clone();
            move || {
                runs.fetch_add(1, Ordering::SeqCst);
                async {}
            }
        };
        assert!(poller.ensure(&tokio::runtime::Handle::current(), connected, run));
        until("又断开后停下", || !poller.running()).await;
        assert_eq!(runs.load(Ordering::SeqCst), 2);
    }
}
