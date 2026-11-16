//! `setup_done` 只授权同一次部署、同一个容器启动代；并发请求共享正在执行的检查。

use std::sync::{Arc, Mutex};
use std::time::Duration;

use bollard::models::{ContainerStateStatusEnum, HealthStatusEnum};
use serde_json::json;
use tokio::sync::watch;
use tokio::time::{Instant, timeout_at};

use super::clients_entry::SetupCheck;
use super::manager::{ClientsRoute, PluginManager, PluginStatus};
use super::manifest::PluginId;
use super::models::COMMAND_TIMEOUT;
use crate::l10n;
use crate::matrix::NoticeRoom;

/// 后台重查的间隔（ADR-0022「不确定时按间隔重试」）：请求路径照旧即时查，这里把没有流量的
/// 入口的状态也翻过来。得 1 的持久关闭在检查完成时统一做，不在这里重复。
const RETRY_INTERVAL: Duration = Duration::from_secs(5);

#[derive(Clone, PartialEq, Eq)]
struct Incarnation {
    container: String,
    started: String,
    restarts: i64,
}

struct Flight {
    incarnation: Incarnation,
    result: watch::Receiver<Option<SetupCheck>>,
}

#[derive(Default)]
struct Memory {
    done: Option<Incarnation>,
    flight: Option<Flight>,
}

#[derive(Default)]
pub(super) struct SetupState(Mutex<Memory>);

impl SetupState {
    /// 测试用：当前部署代是否记着一次完成的检查。
    #[cfg(test)]
    pub(super) fn has_done_for_test(&self) -> bool {
        self.0
            .lock()
            .expect("客户端初始化检查锁中毒")
            .done
            .is_some()
    }
}

impl PluginManager {
    pub(crate) async fn clients_setup_done(
        self: &Arc<Self>,
        id: &PluginId,
        route: &ClientsRoute,
        notices: Option<&Arc<NoticeRoom>>,
    ) -> SetupCheck {
        let deadline = Instant::now() + COMMAND_TIMEOUT;
        let Some(state) = self.clients_setup_state(id, route) else {
            return SetupCheck::Unavailable;
        };
        let incarnation = match self
            .clients_incarnation(id, &route.http.service, deadline)
            .await
        {
            Ok(Some(incarnation)) => incarnation,
            Ok(None) => return SetupCheck::Unavailable,
            Err(error) => return SetupCheck::Undetermined(error),
        };
        let mut result = {
            let mut memory = state.0.lock().expect("客户端初始化检查锁中毒");
            if memory.done.as_ref() == Some(&incarnation) {
                return SetupCheck::Done;
            }
            if let Some(flight) = &memory.flight
                && flight.incarnation == incarnation
                && flight.result.borrow().is_none()
            {
                flight.result.clone()
            } else {
                let (sender, receiver) = watch::channel(None);
                memory.flight = Some(Flight {
                    incarnation: incarnation.clone(),
                    result: receiver.clone(),
                });
                let manager = self.clone();
                let id = id.clone();
                let route = route.clone();
                let state = state.clone();
                let notices = notices.cloned();
                // 请求关闭或取消不能遗弃已创建的 Docker exec，再让下一个请求重复启动一份。
                tokio::spawn(async move {
                    let result = manager
                        .run_clients_setup(
                            &id,
                            &route,
                            &state,
                            &incarnation,
                            deadline,
                            notices.as_ref(),
                        )
                        .await;
                    sender.send_replace(Some(result));
                });
                receiver
            }
        };
        loop {
            if let Some(result) = result.borrow_and_update().clone() {
                return result;
            }
            if result.changed().await.is_err() {
                return SetupCheck::Unavailable;
            }
        }
    }

    fn clients_setup_state(&self, id: &PluginId, route: &ClientsRoute) -> Option<Arc<SetupState>> {
        let plugins = self.plugins.read().expect("插件表锁中毒");
        let entry = plugins.get(id)?;
        let clients = entry.clients.as_ref()?;
        (matches!(
            entry.status,
            PluginStatus::Running | PluginStatus::Degraded { .. }
        ) && entry.serving.contains(&route.http.service)
            && clients == route)
            .then(|| entry.setup.clone())
    }

    async fn run_clients_setup(
        &self,
        id: &PluginId,
        route: &ClientsRoute,
        state: &Arc<SetupState>,
        incarnation: &Incarnation,
        deadline: Instant,
        notices: Option<&Arc<NoticeRoom>>,
    ) -> SetupCheck {
        let Some(app) = &route.http.auth.app else {
            return SetupCheck::Unavailable;
        };
        let output = timeout_at(
            deadline,
            self.engine.exec(
                &incarnation.container,
                &app.setup_done,
                None,
                COMMAND_TIMEOUT,
            ),
        )
        .await;
        let result = match output {
            Ok(Ok(output)) if output.exit_code == 0 => SetupCheck::Done,
            Ok(Ok(output)) if output.exit_code == 1 => SetupCheck::Pending,
            Ok(Ok(output)) => SetupCheck::Undetermined(output.stderr),
            Ok(Err(error)) => SetupCheck::Undetermined(error.to_string()),
            Err(_) => SetupCheck::Undetermined(self.clients_setup_timeout()),
        };
        let current = self
            .clients_incarnation(id, &route.http.service, deadline)
            .await;
        // 读锁必须在 await 之前归还：guard 不是 Send，会堵住后台检查任务的 spawn。
        let still_current = {
            let plugins = self.plugins.read().expect("插件表锁中毒");
            plugins.get(id).is_some_and(|entry| {
                Arc::ptr_eq(&entry.setup, state)
                    && matches!(
                        entry.status,
                        PluginStatus::Running | PluginStatus::Degraded { .. }
                    )
                    && entry.serving.contains(&route.http.service)
                    && entry.clients.as_ref() == Some(route)
            })
        };
        if !still_current || !matches!(&current, Ok(Some(current)) if current == incarnation) {
            return SetupCheck::Unavailable;
        }
        if matches!(result, SetupCheck::Done) {
            state.0.lock().expect("客户端初始化检查锁中毒").done = Some(incarnation.clone());
            self.mark_setup_done_ever(id).await;
        }
        // 确认还是当前部署、当前启动代之后才动开关：旧代的检查不能关掉新入口。
        if matches!(result, SetupCheck::Pending) {
            self.close_clients_entries(id, notices).await;
        }
        result
    }

    /// 检查得 0：记下「这个应用注册过管理员」。之后得 1 才算数据被清空（关闭分支）；
    /// 从没注册过的应用得 1 只是主人还没注册，入口保持开（ADR-0022）。只在翻转时写盘。
    async fn mark_setup_done_ever(&self, id: &PluginId) {
        // 读改写持 lan.pass：settings、permissions、lan 等写方都用它互斥，绕开会把主人的改动丢掉。
        let _pass = self.lan.pass.lock().await;
        let mut record = match self.store.load(id) {
            Ok(Some(record)) => record,
            Ok(None) => return,
            Err(error) => {
                tracing::error!(plugin = %id, %error, "Cannot load the record to mark its completed setup");
                return;
            }
        };
        if record.setup_done_ever {
            return;
        }
        record.setup_done_ever = true;
        if let Err(error) = self.store.save(&record) {
            tracing::error!(plugin = %id, %error, "Cannot persist the completed first-time setup");
        }
    }

    /// 注册没完成（退出码 1）：数据被清空或还没初始化，第一个账号可能被外人抢注。持久关闭
    /// 这个插件的 LAN/public 入口并通知主人（ADR-0022）；之后命令返回 0 也不自动重开，
    /// 重新打开是 C7 的开关界面。从没注册过管理员的（`setup_done_ever` 为假）得 1 不是数据
    /// 被清空：入口保持开、请求回 409，主人去注册。记录写失败时不动开关也不通知：下一次
    /// 检查（请求或后台重查）会再来，通知不能先于生效的关闭。
    async fn close_clients_entries(&self, id: &PluginId, notices: Option<&Arc<NoticeRoom>>) {
        let closed = crate::clients::state::ClientsEntries::default();
        {
            // 读改写持 lan.pass（顺序同 lan_pass：先 lan.pass 再插件表锁），清缓存也在同一把锁里。
            let _pass = self.lan.pass.lock().await;
            let mut record = match self.store.load(id) {
                Ok(Some(record)) => record,
                Ok(None) => return,
                Err(error) => {
                    tracing::error!(
                        plugin = %id, %error, "Cannot load the record to close its client entries"
                    );
                    return;
                }
            };
            if record.clients_entries == closed || !record.setup_done_ever {
                return;
            }
            record.clients_entries = closed;
            if let Err(error) = self.store.save(&record) {
                tracing::error!(plugin = %id, %error, "Cannot persist the closed client entries");
                return;
            }
            self.changed();
            self.forget_clients_setup(id);
        }
        let text = l10n::text(
            &self.owner_languages(),
            "gateway-entry-setup-closed",
            &[("id", id.as_str().into())],
        );
        match notices {
            Some(notices) => {
                if let Err(error) = notices
                    .send(&json!({"msgtype": "m.text", "body": text}))
                    .await
                {
                    tracing::error!(
                        plugin = %id, error = %error, "Cannot tell the owner about the closed entries"
                    );
                }
            }
            None => {
                tracing::warn!(
                    plugin = %id, "No Matrix session: the owner is not told about the closed entries"
                )
            }
        }
    }

    /// 关闭入口后清掉该部署的检查缓存：重开开关（C7）后首个请求不能命中旧的 Done 跳过重验。
    /// 锁序同 lan_pass：先 lan.pass 再插件表读锁。
    fn forget_clients_setup(&self, id: &PluginId) {
        let plugins = self.plugins.read().expect("插件表锁中毒");
        if let Some(entry) = plugins.get(id) {
            entry.setup.0.lock().expect("客户端初始化检查锁中毒").done = None;
        }
    }

    /// 开关还开着、状态还没确定的入口：后台重查的对象。开关关了的不查——得 1 已经关闭过，
    /// 重开由 C7 的界面负责，命令回到 0 也不自动重开。
    fn unknown_clients_setups(&self) -> Vec<(PluginId, ClientsRoute)> {
        let plugins = self.plugins.read().expect("插件表锁中毒");
        plugins
            .iter()
            .filter_map(|(id, entry)| {
                let clients = entry.clients.as_ref()?;
                clients.http.auth.app.as_ref()?;
                let record = self.store.load(id).ok().flatten()?;
                (record.clients_entries.lan || record.clients_entries.public)
                    .then(|| (id.clone(), clients.clone()))
            })
            .collect()
    }

    /// 后台重查一轮：未知状态按间隔翻过来（ADR-0022），得 1 由检查路径统一关闭并通知。
    pub(crate) async fn recheck_clients_setup(self: &Arc<Self>, notices: Option<&Arc<NoticeRoom>>) {
        for (id, route) in self.unknown_clients_setups() {
            if let SetupCheck::Undetermined(reason) =
                self.clients_setup_done(&id, &route, notices).await
            {
                tracing::warn!(plugin = %id, "{reason}");
            }
        }
    }

    /// ADR-0022 的后台重查任务：每 [`RETRY_INTERVAL`] 扫一遍还没确定的入口。单飞保证同一个
    /// 部署代同时只有一条命令，请求路径的检查和这里共用。
    pub fn spawn_clients_setup_watch(self: &Arc<Self>, notices: Option<Arc<NoticeRoom>>) {
        let this = self.clone();
        tokio::spawn(async move {
            let mut ticks = tokio::time::interval(RETRY_INTERVAL);
            ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                ticks.tick().await;
                let this = this.clone();
                let notices = notices.clone();
                tokio::spawn(async move { this.recheck_clients_setup(notices.as_ref()).await });
            }
        });
    }

    async fn clients_incarnation(
        &self,
        id: &PluginId,
        service: &str,
        deadline: Instant,
    ) -> Result<Option<Incarnation>, String> {
        let container = self.cfg.instance.container(&id.slug(), service);
        let info = timeout_at(deadline, self.engine.inspect_container(&container))
            .await
            .map_err(|_| self.clients_setup_timeout())?
            .map_err(|error| error.to_string())?;
        let Some(info) = info else { return Ok(None) };
        let Some(state) = info.state else {
            return Ok(None);
        };
        if state.running != Some(true)
            || state.status != Some(ContainerStateStatusEnum::RUNNING)
            || state.paused == Some(true)
            || state.restarting == Some(true)
            || state
                .health
                .and_then(|health| health.status)
                .is_some_and(|health| {
                    !matches!(health, HealthStatusEnum::HEALTHY | HealthStatusEnum::EMPTY)
                })
        {
            return Ok(None);
        }
        let (Some(container), Some(started), Some(restarts)) =
            (info.id, state.started_at, info.restart_count)
        else {
            return Ok(None);
        };
        if container.is_empty() || started.is_empty() || started.starts_with("0001-") {
            return Ok(None);
        }
        Ok(Some(Incarnation {
            container,
            started,
            restarts,
        }))
    }

    fn clients_setup_timeout(&self) -> String {
        crate::l10n::text(&self.owner_languages(), "gateway-entry-setup-timeout", &[])
    }
}
