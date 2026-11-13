//! 推送跟着服务器走（ADR-0042 的“推送带上服务器 ID”）：推送来自哪台，就在哪台的作用域里取消息；推送地址变了，
//! 其他几台也要重新登记。单独放一个文件，`client.rs` 不超过行数上限。

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use wonton_l10n::l10n;
use wonton_protocol::{Answer, ChatFailure, CoreError, PushEndpoint, PushNotification, PushStatus};

use super::ClientCore;
use super::switch::lock;
use crate::l10n::{CATALOG, language};

/// 推送来自一台已经从手机上移除的服务器：动作没有可去的地方。
fn removed() -> CoreError {
    CoreError::NotReady {
        reason: l10n!(CATALOG, language(), "servers-push-from-removed"),
    }
}
use crate::push::{Payload, record};
use crate::scope::{Entered, Scope, Visits};
use crate::servers::ServerList;

/// 给不是当前这台的服务器重新登记，每台最多等这么久（要建隧道、恢复会话、登记、注册 pusher）。
const REGISTER_WAIT: Duration = Duration::from_secs(60);

/// 正在给哪台后台登记哪个地址。`SetPushEndpoint` 每次启动壳都会再来一遍（Android 的 UnifiedPush 开机时常常
/// 连着回调几次 `onNewEndpoint`），而另一台的登记要等 PUT 返回、注册完 pusher 才写盘（`record::save`）。
/// 只靠磁盘记录去重的话，写盘前窗口里的重复地址会登记第二次；内存里记下在登记的地址，和磁盘记录一起去重。
/// 登记结束（成败都算）就清掉，失败的下回照常再登记。
pub(super) type InFlightRegistrations = Arc<Mutex<HashMap<String, PushEndpoint>>>;

enum Route {
    Current(Arc<Scope>),
    Visit(Entered),
    /// 推送里的服务器不在列表里：从这台手机上移除了，那边没吊销这台设备，还在推。
    Removed,
}

#[uniffi::export]
impl ClientCore {
    /// 处理收到的一条推送：`payload` 是壳在本机解开后的明文（Android 上 UnifiedPush 连接库已经按 RFC 8291 解开）。
    /// 核心经隧道取回那条消息、在本机解密，给出要弹出的通知；取不到正文时通知里照实写原因。推送唤醒的进程可以
    /// 一创建核心就调用，核心等会话从本机恢复。不用显示（按推送规则不该响、消息已撤回）时返回空，内容不是家里
    /// 推送代理发的格式时返回 `InvalidInput`。最多等二三十秒。
    ///
    /// 推送来自不是当前连着的服务器时，临时连上那台去取，当前这台不受影响；通知的 `server_id` 是那台。
    pub async fn resolve_push(
        &self,
        payload: Vec<u8>,
    ) -> Result<Option<PushNotification>, CoreError> {
        let payload = Payload::parse(&payload)?;
        let (slot, notification) = match self.route(&payload.server)? {
            Route::Current(scope) => (scope.slot.clone(), in_scope(&scope, &payload).await?),
            Route::Visit(entered) => {
                let visit = entered.visit.clone();
                let push = visit.push.clone();
                let resolving = payload.clone();
                let resolved = visit
                    .runtime
                    .run(async move { push.resolve(&resolving).await })
                    .await;
                drop(entered);
                match resolved {
                    // 取的时候主人正好切到了这台：临时作用域被关掉了，改在当前的作用域里取。
                    Err(CoreError::NotReady { .. }) if self.scope().slot == visit.slot => {
                        let scope = self.scope();
                        (scope.slot.clone(), in_scope(&scope, &payload).await?)
                    }
                    resolved => (visit.slot.clone(), resolved?),
                }
            }
            Route::Removed => {
                return Ok(Some(payload.unavailable(
                    ChatFailure::NotReady,
                    l10n!(CATALOG, language(), "servers-push-from-removed"),
                )));
            }
        };
        Ok(notification.map(|notification| PushNotification {
            server_id: Some(slot),
            ..notification
        }))
    }

    /// 通知上的“允许/拒绝”（ADR-0101 §1）：和卡片按钮走同一条回答路径，等结果——成败要写回通知，日志里
    /// 看不出主人点过。`server_id` 是通知里的那台：来自别的服务器时，在它的临时作用域里答完再回来。进程是推送
    /// 唤醒的、App 没开时也能调；连不上就报 `NotReady`/`TransportUnavailable`，不排队假装答上了。
    pub async fn answer_push_confirmation(
        &self,
        server_id: Option<String>,
        card_id: String,
        answer: Answer,
        choice_id: Option<String>,
    ) -> Result<(), CoreError> {
        match self.route_push_server(server_id.as_deref())? {
            Route::Current(scope) => {
                let chat = scope.chat.clone();
                scope
                    .run(async move { chat.answer_from_push(card_id, answer, choice_id).await })
                    .await
            }
            Route::Visit(entered) => {
                let visit = entered.visit.clone();
                let chat = visit.chat.clone();
                let result = visit
                    .runtime
                    .run(async move { chat.answer_from_push(card_id, answer, choice_id).await })
                    .await;
                drop(entered);
                result
            }
            Route::Removed => Err(removed()),
        }
    }

    /// 通知回复先校验目标，再存进待发队列；成功表示已入队，送达或失败由时间线显示。不动草稿和附件区。
    pub async fn reply_to_push(
        &self,
        server_id: Option<String>,
        room_id: String,
        thread_root: Option<String>,
        reply_to: Option<String>,
        text: String,
    ) -> Result<(), CoreError> {
        match self.route_push_server(server_id.as_deref())? {
            Route::Current(scope) => {
                let chat = scope.chat.clone();
                let composer = scope.composer.clone();
                scope
                    .run(async move {
                        let reply = chat
                            .prepare_push_reply(
                                &room_id,
                                &text,
                                reply_to.as_deref(),
                                thread_root.as_deref(),
                            )
                            .await?;
                        composer.send_push_reply(&room_id, &text, reply)
                    })
                    .await
            }
            Route::Visit(entered) => {
                let visit = entered.visit.clone();
                let chat = visit.chat.clone();
                let composer = visit.composer()?;
                let sending_room = room_id.clone();
                let result = visit
                    .runtime
                    .run(async move {
                        let reply = chat
                            .prepare_push_reply(
                                &sending_room,
                                &text,
                                reply_to.as_deref(),
                                thread_root.as_deref(),
                            )
                            .await?;
                        composer.send_push_reply(&sending_room, &text, reply)
                    })
                    .await;
                if result.is_ok() {
                    let composer = visit.composer()?;
                    let chat = visit.chat.clone();
                    // 入队不是送达：后台槽位留出发送窗口，不影响当前页面，也不无限持有另一台的作用域。
                    visit.runtime.spawn(async move {
                        let _using = entered;
                        let _ = tokio::time::timeout(Duration::from_secs(30), async {
                            composer.wait_for_handover(&room_id).await;
                            if let Ok(session) = chat.current()
                                && let Ok(room) = room_id.parse::<wonton_matrix::OwnedRoomId>()
                            {
                                let _ = session.wait_for_queued_sends(&room).await;
                            }
                        })
                        .await;
                    });
                }
                result
            }
            Route::Removed => Err(removed()),
        }
    }
}

impl ClientCore {
    /// 和换服务器串行：判断是不是当前这台、为它开临时作用域，中间不会换服务器。
    fn route(&self, server: &str) -> Result<Route, CoreError> {
        let _switching = lock(&self.switching);
        let Some(slot) = self.servers.slot_of(server) else {
            return Ok(Route::Removed);
        };
        let current = self.scope();
        if current.slot == slot {
            return Ok(Route::Current(current));
        }
        Ok(Route::Visit(self.visits.enter(&slot)?))
    }

    /// 通知保存的是本机槽位，不是服务器 endpoint；缺失或已移除的槽位绝不能落到当前服务器。
    fn route_push_server(&self, server_id: Option<&str>) -> Result<Route, CoreError> {
        let _switching = lock(&self.switching);
        let Some(slot) = server_id else {
            return Ok(Route::Removed);
        };
        if !self
            .servers
            .view()
            .servers
            .iter()
            .any(|server| server.id == slot)
        {
            return Ok(Route::Removed);
        }
        let current = self.scope();
        if current.slot == slot {
            return Ok(Route::Current(current));
        }
        Ok(Route::Visit(self.visits.enter(slot)?))
    }

    /// 推送地址变了：配过的其他几台在后台逐台临时连上去重新登记。连不上的，下次切过去时当前作用域会补上。
    /// 地址没变（每次启动壳都会再给一遍）的不动。
    pub(super) fn register_others(&self, scope: &Scope, endpoint: PushEndpoint) {
        let others = others_to_register(
            &self.servers,
            &self.in_flight_registrations,
            &scope.slot,
            &endpoint,
        );
        if others.is_empty() {
            return;
        }
        let visits = self.visits.clone();
        let in_flight = self.in_flight_registrations.clone();
        scope.spawn(async move {
            for slot in others {
                register_on(&visits, &in_flight, &slot, endpoint.clone()).await;
            }
        });
    }
}

/// 已落盘的记录相同、或者正在登记的地址相同，都不用再登记。查“在途”和记下“要登记”要在同一把锁里做完：
/// 分开的话，挨得极近的两次同一地址会在第一次记下之前都放进来。
fn others_to_register(
    servers: &ServerList,
    in_flight: &InFlightRegistrations,
    current: &str,
    endpoint: &PushEndpoint,
) -> Vec<String> {
    let candidates: Vec<String> = servers
        .view()
        .servers
        .into_iter()
        .map(|row| row.id)
        .filter(|slot| slot != current && !record::matches(&servers.slot_dir(slot), endpoint))
        .collect();
    let mut in_flight = lock(in_flight);
    candidates
        .into_iter()
        .filter(|slot| {
            if in_flight.get(slot) == Some(endpoint) {
                return false;
            }
            in_flight.insert(slot.clone(), endpoint.clone());
            true
        })
        .collect()
}

async fn register_on(
    visits: &Arc<Visits>,
    in_flight: &InFlightRegistrations,
    slot: &str,
    endpoint: PushEndpoint,
) {
    let registered = register_on_another(visits, slot, endpoint.clone()).await;
    // 登记结束（成败都算）就清掉这条“正在登记”；条目已经被后来的新地址顶掉的话不动，那边有自己的登记要清。
    {
        let mut in_flight = lock(in_flight);
        if in_flight.get(slot) == Some(&endpoint) {
            in_flight.remove(slot);
        }
    }
    match registered {
        Ok(Some(PushStatus::Registered)) => {
            tracing::info!(slot, "re-registered push for another server")
        }
        Ok(Some(status)) => tracing::info!(
            slot,
            ?status,
            "could not re-register push for another server"
        ),
        // 没做登记：已经切到这台（当前作用域会登记），或者没能临时连上（内层已记日志）。
        Ok(None) => {}
        Err(error) => tracing::info!(slot, %error, "could not re-register push for another server"),
    }
}

async fn register_on_another(
    visits: &Arc<Visits>,
    slot: &str,
    endpoint: PushEndpoint,
) -> Result<Option<PushStatus>, CoreError> {
    let entered = match visits.other(slot) {
        Ok(Some(entered)) => entered,
        // 已经切到这台了，当前作用域会登记。
        Ok(None) => return Ok(None),
        Err(error) => {
            tracing::warn!(%error, slot, "could not open another server to re-register push");
            return Ok(None);
        }
    };
    let chat = entered.visit.chat.clone();
    let push = entered.visit.push.clone();
    entered
        .visit
        .runtime
        .run(async move {
            let mut view = chat.subscribe();
            push.set_endpoint(endpoint)?;
            let settled = view.wait_for(|view| {
                matches!(
                    view.push,
                    PushStatus::Registered | PushStatus::Failed { .. }
                )
            });
            match tokio::time::timeout(REGISTER_WAIT, settled).await {
                Ok(Ok(view)) => Ok(Some(view.push.clone())),
                Ok(Err(_)) => Err(CoreError::NotReady {
                    reason: l10n!(CATALOG, language(), "servers-push-scope-closed"),
                }),
                Err(_) => Err(CoreError::TransportUnavailable {
                    reason: l10n!(
                        CATALOG,
                        language(),
                        "servers-push-register-timeout",
                        seconds = REGISTER_WAIT.as_secs()
                    ),
                }),
            }
        })
        .await
}

async fn in_scope(scope: &Scope, payload: &Payload) -> Result<Option<PushNotification>, CoreError> {
    let push = scope.push.clone();
    let payload = payload.clone();
    scope.run(async move { push.resolve(&payload).await }).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_or_removed_notification_target_never_uses_selected_scope() {
        let dir = tempfile::tempdir().unwrap();
        let core = ClientCore::new(dir.path().to_string_lossy().into_owned()).unwrap();
        assert!(matches!(
            core.route_push_server(None).unwrap(),
            Route::Removed
        ));
        assert!(matches!(
            core.route_push_server(Some("deleted")).unwrap(),
            Route::Removed
        ));
    }
}
