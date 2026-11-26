//! 待完成的签发、换值和吊销（ADR-0021「专用密码」）：先落盘再跑命令，退出码 0 才落地。
//!
//! 插件没在跑、命令失败或超时，记录都停在待完成：值照旧能用，下次插件启动先只重试那一个操作，
//! 再对账其余的。对账绝不把已经被待换值取代的旧值再发一遍。

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use super::credentials::{CredentialError, Verify};
use super::passwords::{self, PasswordError, PasswordStore, Snapshot, State};

use wonton_protocol::PendingOp;
use wonton_protocol::manifest::CredentialsAuth;

use crate::l10n;
use crate::plugin::PluginId;

/// 待完成的凭据操作多久重试一次（ADR-0021「吊销不能丢」）。
const RETRY_INTERVAL: Duration = Duration::from_secs(30);

/// 新建密码的结果：还没发成的记录也已落盘，值照旧给主人。
#[derive(Debug)]
pub(crate) struct Issued {
    pub id: String,
    pub value: String,
}

/// 新建的三种结果。
#[derive(Debug)]
pub(crate) enum Issue {
    /// 应用里已经生效。
    Done(Issued),
    /// 插件没在跑：记录待完成，启动时重试；回 202。
    Deferred(Issued),
    /// 命令失败或超时：记录待完成、值留着，回 502。
    Failed {
        issued: Issued,
        error: CredentialError,
    },
}

/// 吊销/换值的结果。
#[derive(Debug)]
pub(crate) enum Settle {
    /// 执行完了（吊销的已删除、换值的已生效）。
    Done,
    /// 插件没在跑，留在待完成等启动或后台重试。
    Deferred(PendingOp),
    /// 失败或超时，留在待完成等下次。
    Failed(CredentialError),
}

/// 吊销一台设备时的结果：没完成的应用和登录没结束的应用，按插件 ID 去重。
#[derive(Debug, Default)]
pub(crate) struct DeviceRevokeReport {
    pub pending_revokes: Vec<String>,
    pub ends_sessions_false: Vec<String>,
}

/// 新建密码时的问题；`PasswordError` 单独包着。
#[derive(Debug)]
pub(crate) enum IssueError {
    Store,
    SharedExists,
    Password(PasswordError),
}

impl From<PasswordError> for IssueError {
    fn from(error: PasswordError) -> Self {
        Self::Password(error)
    }
}

impl crate::plugin::PluginManager {
    /// 新建一个密码并立刻下发。`per_client: false` 的插件已经有密码时回 [`IssueError::SharedExists`]。
    pub(crate) async fn issue_new(
        &self,
        plugin: &PluginId,
        label: &str,
        device: Option<(&str, Option<&str>)>,
    ) -> Result<Issue, IssueError> {
        let Some(store) = &self.client_passwords else {
            return Err(IssueError::Store);
        };
        let _serial = self.credential_lock(plugin).await;
        let Some(credentials) = self.credentials_auth(plugin) else {
            // 网关验证（C16）没有命令：值直接落盘就能用。
            let created = store.create(plugin.as_str(), label, device)?;
            return Ok(Issue::Done(Issued {
                id: created.id,
                value: created.value,
            }));
        };
        let created = if credentials.per_client {
            if credentials.generates {
                store.create_generated_pending(plugin.as_str(), label, device)?
            } else {
                store.create_pending(plugin.as_str(), label, device, passwords::random_value())?
            }
        } else {
            // 共享：固定 ID `shared`，检查和插入在同一把锁里，并发 POST 只有一个成功。
            let (value, next) = if credentials.generates {
                (Some(String::new()), None)
            } else {
                let value = passwords::random_value();
                (Some(value.clone()), Some(value))
            };
            let Some(created) = store.ensure_shared(plugin.as_str(), label, device, value, next)?
            else {
                return Err(IssueError::SharedExists);
            };
            created
        };
        let snapshot = store.snapshot(plugin.as_str(), &created.id)?;
        let issued = Issued {
            id: created.id.clone(),
            value: created.value,
        };
        let outcome = self
            .issue_password(plugin, &snapshot, credentials.generates)
            .await;
        match outcome {
            Ok(generated) => {
                store.finish_rotate(
                    plugin.as_str(),
                    &created.id,
                    generated,
                    credentials.ends_sessions,
                )?;
                store.mark_issued(plugin.as_str())?;
                let value = store.snapshot(plugin.as_str(), &created.id)?.value;
                Ok(Issue::Done(Issued {
                    id: created.id,
                    value,
                }))
            }
            Err(CredentialError::Unavailable) => Ok(Issue::Deferred(issued)),
            Err(error) => Ok(Issue::Failed { issued, error }),
        }
    }

    /// 吊销一个密码；`per_client: false` 时其实是换值。
    pub(crate) async fn settle_password(
        &self,
        plugin: &PluginId,
        id: &str,
    ) -> Result<Settle, PasswordError> {
        let Some(store) = &self.client_passwords else {
            return Err(PasswordError::NotFound);
        };
        let _serial = self.credential_lock(plugin).await;
        let snapshot = store.snapshot(plugin.as_str(), id)?;
        let Some(credentials) = self.credentials_auth(plugin) else {
            store.revoke(plugin.as_str(), id)?;
            return Ok(Settle::Done);
        };
        if credentials.per_client {
            self.revoke_record(plugin, &snapshot, credentials.ends_sessions)
                .await
        } else {
            self.rotate_record(plugin, &snapshot, &credentials).await
        }
    }

    async fn revoke_record(
        &self,
        plugin: &PluginId,
        snapshot: &Snapshot,
        ends_sessions: bool,
    ) -> Result<Settle, PasswordError> {
        let store = self.client_passwords.as_ref().expect("调用方查过密码库");
        store.begin_revoke(plugin.as_str(), &snapshot.id)?;
        match self.revoke_password(plugin, snapshot).await {
            Ok(()) => {
                store.finish_revoke(plugin.as_str(), &snapshot.id, ends_sessions)?;
                Ok(Settle::Done)
            }
            Err(CredentialError::Unavailable) => Ok(Settle::Deferred(PendingOp::Revoke)),
            Err(error) => Ok(Settle::Failed(error)),
        }
    }

    /// 换值；`generates` 时新值由应用写回。旧值在新命令成功前照旧能用。
    async fn rotate_record(
        &self,
        plugin: &PluginId,
        snapshot: &Snapshot,
        credentials: &CredentialsAuth,
    ) -> Result<Settle, PasswordError> {
        let store = self.client_passwords.as_ref().expect("调用方查过密码库");
        let next = (!credentials.generates).then(passwords::random_value);
        store.begin_rotate(plugin.as_str(), &snapshot.id, next.clone())?;
        match self
            .rotate_password(plugin, snapshot, next.as_deref())
            .await
        {
            Ok(generated) => {
                store.finish_rotate(
                    plugin.as_str(),
                    &snapshot.id,
                    generated,
                    credentials.ends_sessions,
                )?;
                store.mark_issued(plugin.as_str())?;
                Ok(Settle::Done)
            }
            Err(CredentialError::Unavailable) => Ok(Settle::Deferred(PendingOp::Rotate)),
            Err(error) => Ok(Settle::Failed(error)),
        }
    }

    /// 设备吊销：这台设备建过或看过的密码跟着走。
    ///
    /// `per_client: false` 的共享密码默认换值（别台设备继续用新值）；`keep_shared` 时原样留着，
    /// 别的设备的老值照旧能用、连接也不断，下次启动只按对账重发同一个值。
    /// `ends_sessions: false` 的插件登录没结束，单独列出。没被这台设备碰过的不动。
    pub(crate) async fn revoke_passwords_for_device(
        &self,
        device: &str,
        keep_shared: bool,
    ) -> Result<DeviceRevokeReport, PasswordError> {
        let Some(store) = &self.client_passwords else {
            return Ok(DeviceRevokeReport::default());
        };
        let mut report = DeviceRevokeReport::default();
        let mut pending = BTreeSet::new();
        let mut kept_sessions = BTreeSet::new();
        for (plugin, id) in store.for_device(device) {
            let Ok(parsed) = PluginId::parse(&plugin) else {
                store.revoke(&plugin, &id)?;
                continue;
            };
            let _serial = self.credential_lock(&parsed).await;
            let snapshot = match store.snapshot(&plugin, &id) {
                Ok(snapshot) => snapshot,
                Err(PasswordError::NotFound) => continue,
                Err(error) => return Err(error),
            };
            let Some(credentials) = self.credentials_auth(&parsed) else {
                store.revoke(&plugin, &id)?;
                continue;
            };
            if !credentials.ends_sessions {
                kept_sessions.insert(plugin.clone());
            }
            let settle = if credentials.per_client {
                self.revoke_record(&parsed, &snapshot, credentials.ends_sessions)
                    .await
            } else if keep_shared {
                // 共享模式没有 revoke 命令：记录和值都留着，不动守卫。
                Ok(Settle::Done)
            } else {
                self.rotate_record(&parsed, &snapshot, &credentials).await
            };
            match settle {
                Ok(Settle::Done) => {}
                Ok(Settle::Deferred(_) | Settle::Failed(_)) => {
                    pending.insert(plugin.clone());
                }
                Err(error) => return Err(error),
            }
        }
        report.pending_revokes = pending.into_iter().collect();
        report.ends_sessions_false = kept_sessions.into_iter().collect();
        Ok(report)
    }

    /// 同一插件的凭据操作串行跑。
    pub(crate) async fn credential_lock(
        &self,
        plugin: &PluginId,
    ) -> tokio::sync::OwnedMutexGuard<()> {
        let lock = self
            .credential_locks
            .lock()
            .expect("credential lock table poisoned")
            .entry(plugin.clone())
            .or_default()
            .clone();
        lock.lock_owned().await
    }

    /// 重试一条待换：有 rotate 跑 rotate、把待换值给它；没有 rotate 就用 issue 覆盖，
    /// 发的也是待换的新值，绝不是被取代的旧值（`generates` 由应用出值，不给标准输入）。
    async fn retry_pending_rotate(
        &self,
        plugin: &PluginId,
        store: &PasswordStore,
        snapshot: &Snapshot,
        credentials: &CredentialsAuth,
        next: Option<&str>,
    ) {
        let outcome = if credentials.rotate.is_some() {
            self.rotate_password(plugin, snapshot, next).await
        } else {
            let generates = credentials.generates;
            let mut pending = snapshot.clone();
            if let Some(next) = next {
                pending.value = next.to_owned();
            }
            self.issue_password(plugin, &pending, generates).await
        };
        match outcome {
            Ok(generated) => {
                if let Err(error) = store.finish_rotate(
                    plugin.as_str(),
                    &snapshot.id,
                    generated,
                    credentials.ends_sessions,
                ) {
                    tracing::warn!(plugin = %plugin, "记下换值失败：{error}");
                } else if let Err(error) = store.mark_issued(plugin.as_str()) {
                    tracing::warn!(plugin = %plugin, "记下发过密码失败：{error}");
                }
            }
            Err(error) => tracing::warn!(plugin = %plugin, "重试换值失败：{error}"),
        }
    }

    /// 插件启动成功后：补共享密码，先只重试待完成的操作，再对账已落地的值。
    ///
    /// 失败只记日志：启动已经成功，不能因为应用里的命令坏了再把插件拖回去。
    pub(crate) async fn reconcile_credentials(&self, plugin: &PluginId) {
        let _serial = self.credential_lock(plugin).await;
        let Some(store) = &self.client_passwords else {
            return;
        };
        let Some(credentials) = self.credentials_auth(plugin) else {
            return;
        };
        if !credentials.per_client {
            let label = l10n::text(&self.owner_languages(), "clients-password-shared", &[]);
            let (value, next) = if credentials.generates {
                (Some(String::new()), None)
            } else {
                let value = passwords::random_value();
                (Some(value.clone()), Some(value))
            };
            match store.ensure_shared(plugin.as_str(), &label, None, value, next) {
                Ok(Some(_)) | Ok(None) => {}
                Err(error) => tracing::warn!(plugin = %plugin, "写共享密码失败：{error}"),
            }
        }
        self.retry_pendings(plugin, store, &credentials).await;
        // 再对账已落地的值：有 verify 先验，无效或没有 verify 才重新发；未知先不动。
        for snapshot in store.snapshots(plugin.as_str()) {
            if snapshot.state != State::Ready {
                continue;
            }
            if credentials.verify.is_some() {
                match self.verify_password(plugin, &snapshot).await {
                    Verify::Valid => {
                        let _ = store.mark_issued(plugin.as_str());
                        continue;
                    }
                    Verify::Unknown => continue,
                    Verify::Invalid => {}
                }
            }
            let result = if credentials.generates {
                // 应用自己出值：先落待换，命令失败/超时时应用可能已经换了新值，记录不能留旧值。
                if let Err(error) = store.begin_rotate(plugin.as_str(), &snapshot.id, None) {
                    tracing::warn!(plugin = %plugin, "记下待换值失败：{error}");
                    continue;
                }
                let outcome = if credentials.rotate.is_some() {
                    self.rotate_password(plugin, &snapshot, None).await
                } else {
                    self.issue_password(plugin, &snapshot, true).await
                };
                match outcome {
                    Ok(generated) => {
                        if let Err(error) =
                            store.finish_rotate(plugin.as_str(), &snapshot.id, generated, false)
                        {
                            tracing::warn!(plugin = %plugin, "记下重新生成的值失败：{error}");
                            continue;
                        }
                        Ok(())
                    }
                    Err(error) => Err(error),
                }
            } else {
                self.issue_password(plugin, &snapshot, false)
                    .await
                    .map(|_| ())
            };
            match result {
                Ok(()) => {
                    if let Err(error) = store.mark_issued(plugin.as_str()) {
                        tracing::warn!(plugin = %plugin, "记下发过密码失败：{error}");
                    }
                }
                Err(error) => tracing::warn!(plugin = %plugin, "对账密码失败：{}", error.detail()),
            }
        }
    }
    /// 重试某个插件所有没完成的操作：待吊销只重试吊销，待换只重试换值。调用方持锁。
    async fn retry_pendings(
        &self,
        plugin: &PluginId,
        store: &PasswordStore,
        credentials: &CredentialsAuth,
    ) {
        for snapshot in store.snapshots(plugin.as_str()) {
            match snapshot.state {
                State::PendingRevoke => {
                    if let Err(error) = self
                        .revoke_record(plugin, &snapshot, credentials.ends_sessions)
                        .await
                    {
                        tracing::warn!(plugin = %plugin, "重试吊销失败：{error}");
                    }
                }
                State::PendingRotate { ref next } => {
                    self.retry_pending_rotate(
                        plugin,
                        store,
                        &snapshot,
                        credentials,
                        next.as_deref(),
                    )
                    .await
                }
                State::Ready => {}
            }
        }
    }

    /// 后台重试一个插件：正被别的操作占着就跳过，下一轮再来。
    pub(crate) async fn retry_pending_credentials(&self, plugin: &PluginId) {
        let Some(store) = &self.client_passwords else {
            return;
        };
        let Some(credentials) = self.credentials_auth(plugin) else {
            return;
        };
        let Some(_serial) = self.try_credential_lock(plugin) else {
            return;
        };
        self.retry_pendings(plugin, store, &credentials).await;
    }

    /// ADR-0021「吊销不能丢」的隔一段重试：插件一直运行时，失败或超时的不等到下次启动。
    /// 拿到锁才跑，起一次操作就占着锁，下一轮撞上直接跳过。
    fn try_credential_lock(&self, plugin: &PluginId) -> Option<tokio::sync::OwnedMutexGuard<()>> {
        let lock = self
            .credential_locks
            .lock()
            .expect("credential lock table poisoned")
            .entry(plugin.clone())
            .or_default()
            .clone();
        lock.try_lock_owned().ok()
    }

    /// 每隔 [`RETRY_INTERVAL`] 扫一遍还没完成的凭据操作，每个插件一条任务。
    pub fn spawn_credential_watch(self: &Arc<Self>) {
        let this = self.clone();
        tokio::spawn(async move {
            let mut ticks = tokio::time::interval(RETRY_INTERVAL);
            ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                ticks.tick().await;
                for id in this.plugins_with_pending_credentials() {
                    let this = this.clone();
                    tokio::spawn(async move { this.retry_pending_credentials(&id).await });
                }
            }
        });
    }

    /// 给状态用的待完成操作；值另外取。
    pub(crate) fn pending_op(state: &State) -> Option<PendingOp> {
        match state {
            State::Ready => None,
            State::PendingRotate { .. } => Some(PendingOp::Rotate),
            State::PendingRevoke => Some(PendingOp::Revoke),
        }
    }
}
