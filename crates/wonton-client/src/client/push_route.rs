//! 推送跟着服务器走（ADR-0042 的“推送带上服务器 ID”）：推送来自哪台，就在哪台的作用域里取消息；推送地址变了，
//! 其他几台也要重新登记。单独放一个文件，`client.rs` 不超过行数上限。

use std::sync::Arc;
use std::time::Duration;

use wonton_l10n::l10n;
use wonton_protocol::{ChatFailure, CoreError, PushEndpoint, PushNotification, PushStatus};

use super::ClientCore;
use super::switch::lock;
use crate::l10n::{CATALOG, language};
use crate::push::{Payload, record};
use crate::scope::{Entered, Scope, Visits};
use crate::servers::ServerList;

/// 给不是当前这台的服务器重新登记，每台最多等这么久（要建隧道、恢复会话、登记、注册 pusher）。
const REGISTER_WAIT: Duration = Duration::from_secs(60);

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

    /// 推送地址变了：配过的其他几台在后台逐台临时连上去重新登记。连不上的，下次切过去时当前作用域会补上。
    /// 地址没变（每次启动壳都会再给一遍）的不动。
    pub(super) fn register_others(&self, scope: &Scope, endpoint: PushEndpoint) {
        let others = others_to_register(&self.servers, &scope.slot, &endpoint);
        if others.is_empty() {
            return;
        }
        let visits = self.visits.clone();
        scope.spawn(async move {
            for slot in others {
                register_on(&visits, &slot, endpoint.clone()).await;
            }
        });
    }
}

fn others_to_register(servers: &ServerList, current: &str, endpoint: &PushEndpoint) -> Vec<String> {
    servers
        .view()
        .servers
        .into_iter()
        .map(|row| row.id)
        .filter(|slot| slot != current && !record::matches(&servers.slot_dir(slot), endpoint))
        .collect()
}

async fn register_on(visits: &Arc<Visits>, slot: &str, endpoint: PushEndpoint) {
    let entered = match visits.other(slot) {
        Ok(Some(entered)) => entered,
        // 已经切到这台了，当前作用域会登记。
        Ok(None) => return,
        Err(error) => {
            tracing::warn!(%error, slot, "could not open another server to re-register push");
            return;
        }
    };
    let chat = entered.visit.chat.clone();
    let push = entered.visit.push.clone();
    let registered = entered
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
                Ok(Ok(view)) => Ok(view.push.clone()),
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
        .await;
    match registered {
        Ok(PushStatus::Registered) => tracing::info!(slot, "re-registered push for another server"),
        Ok(status) => tracing::info!(
            slot,
            ?status,
            "could not re-register push for another server"
        ),
        Err(error) => tracing::info!(slot, %error, "could not re-register push for another server"),
    }
}

async fn in_scope(scope: &Scope, payload: &Payload) -> Result<Option<PushNotification>, CoreError> {
    let push = scope.push.clone();
    let payload = payload.clone();
    scope.run(async move { push.resolve(&payload).await }).await
}
