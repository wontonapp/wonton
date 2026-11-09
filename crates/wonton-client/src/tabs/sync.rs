//! 底栏和面板配置往 Matrix 账号数据的同步(ADR-0052):登录取会话、推送本机改动、
//! 跟着别的设备写的变化走。断线重连的会话变了就重来。
use super::*;

impl TabsLink {
    pub(super) fn spawn_watchers(self: &Arc<Self>) {
        let mut plugins = self.plugins_view.clone();
        let mut home = self.home.clone();
        let mut room_panel = self.room_panel.1.clone();
        let mut sync_problem = self.sync_problem.subscribe();
        let weak = Arc::downgrade(self);
        self.runtime.spawn(async move {
            loop {
                tokio::select! {
                    changed = plugins.changed() => if changed.is_err() { break },
                    changed = home.changed() => if changed.is_err() { break },
                    changed = room_panel.changed() => if changed.is_err() { break },
                    changed = sync_problem.changed() => if changed.is_err() { break },
                }
                let Some(link) = weak.upgrade() else { break };
                link.recompute();
            }
        });
        let mut session = self.session.clone();
        let weak = Arc::downgrade(self);
        self.runtime.spawn(async move {
            loop {
                if session.changed().await.is_err() {
                    break;
                }
                let Some(link) = weak.upgrade() else { break };
                link.restart_sync();
            }
        });
    }

    /// 会话来了（或换了）：先和账号数据对一次账，然后跟着它变化。
    fn restart_sync(self: &Arc<Self>) {
        let mut task = lock(&self.sync_task);
        if let Some(task) = task.take() {
            task.abort();
        }
        let Some(session) = self.session.borrow().clone() else {
            // 登出了：本机的算数，登录后再推（旧版 tabs.json 也这样搬进账号数据）。
            return;
        };
        let weak = Arc::downgrade(self);
        *task = Some(self.runtime.spawn(async move {
            let Some(link) = weak.upgrade() else { return };
            if let Err(error) = link.reconcile(&session).await {
                tracing::warn!("could not sync the tab layout: {error}");
                return;
            }
            let Ok(changes) = session.layout_changes().await else {
                link.sync_problem.send_replace(Some(l10n!(
                    CATALOG,
                    language(),
                    "panel-sync-failed-read"
                )));
                return;
            };
            let mut changes = std::pin::pin!(changes);
            while let Some(content) = changes.next().await {
                let Some(link) = weak.upgrade() else { return };
                link.follow_server(content).await;
            }
        }));
    }

    /// 会话刚就绪：账号数据和本机配置对一次账。服务器上有配置、本机也没改过，就收服务器的；
    /// 否则把本机的推上去（服务器上还没有时，本机的配置就这样第一次进了账号数据）。
    async fn reconcile(self: &Arc<Self>, session: &Arc<MatrixSession>) -> Result<(), CoreError> {
        let server = session.server_layout().await.map_err(core_error)?;
        let dirty = self.dirty.load(Ordering::SeqCst);
        match (server, dirty) {
            (Some(content), false) => self.follow_server(content).await,
            _ => {
                let settings = lock(&self.state).config.panel_layout();
                self.push_layout(session, settings).await;
            }
        }
        Ok(())
    }

    /// 服务器上的配置变了：本机有没同步上的改动就让位（推本机的），否则收下服务器这份。
    async fn follow_server(self: &Arc<Self>, content: LayoutContent) {
        if self.dirty.load(Ordering::SeqCst) {
            let session = match self.session.borrow().clone() {
                Some(session) => session,
                None => return,
            };
            let settings = lock(&self.state).config.panel_layout();
            self.push_layout(&session, settings).await;
            return;
        }
        let Ok(settings) = serde_json::from_value::<LayoutSettings>(content.0.into()) else {
            // 账号数据整个读不出（多半是更新版 App 写的新格式）：显示原因，本机的照旧。
            self.sync_problem.send_replace(Some(l10n!(
                CATALOG,
                language(),
                "panel-sync-unreadable"
            )));
            return;
        };
        let (plugins, _, _) = self.inputs();
        let _writing = lock(&self.writing);
        let mut state = lock(&self.state).clone();
        if let Err(reason) = adopt_layout(&mut state.config, settings) {
            self.sync_problem.send_replace(Some(reason));
            return;
        }
        if let Err(error) = persist(&self.path, &state.config) {
            self.sync_problem
                .send_replace(Some(storage_problem(&error).message));
            return;
        }
        let view = make_view(&state, &plugins);
        *lock(&self.state) = state;
        self.publish(view, &plugins);
    }

    /// 把本机的配置写上账号数据。写成了清掉“没同步上”；写不上留着，下次再推。
    pub(super) async fn push_layout(&self, session: &Arc<MatrixSession>, settings: LayoutSettings) {
        let written = session
            .update_layout(|raw| overwrite_layout(raw, &settings))
            .await;
        match written {
            Ok(_) => {
                self.dirty.store(false, Ordering::SeqCst);
                self.sync_problem.send_replace(None);
            }
            Err(error) => {
                self.sync_problem.send_replace(Some(error.to_string()));
            }
        }
    }
}

/// 账号数据出问题按上游算：服务器收了但没存好，或请求没成。
fn core_error(error: wonton_matrix::Error) -> CoreError {
    CoreError::Upstream {
        reason: error.to_string(),
    }
}
