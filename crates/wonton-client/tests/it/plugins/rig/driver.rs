//! 驱动 App 的工具：装上假核心、连上、等状态。

use std::net::{Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use iroh::EndpointId;
use wonton_client::{
    Action, ClientCore, HomeStatus, Network, OnboardingViewModel, PluginTaskState, PluginsViewModel,
};

use super::state::CoreState;
use super::{FakeHome, WAIT, ticket};

pub struct Rig {
    pub core: Arc<CoreState>,
    pub app: Arc<ClientCore>,
    _home: FakeHome,
    pub home_ticket: String,
    _dir: tempfile::TempDir,
}

impl Rig {
    pub async fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let app =
            ClientCore::with_network(dir.path().to_str().unwrap(), Network::DirectOnly).unwrap();
        let device = app.home().device_id.parse().unwrap();
        let home = FakeHome::start(vec![device]).await;
        Self {
            core: home.core.clone(),
            app,
            home_ticket: home.ticket(),
            _home: home,
            _dir: dir,
        }
    }

    /// 关掉隧道的定时探测：繁忙时一次卡满时限的探测会断开健康连接、触发整轮重连，
    /// 插件测试的对象不是隧道恢复。初始建连不受影响。
    pub fn without_periodic_checks(self) -> Self {
        self.app.stop_periodic_checks();
        self
    }

    /// 关掉 App 再打开。
    pub fn reopen(&mut self) {
        self.app =
            ClientCore::with_network(self._dir.path().to_str().unwrap(), Network::DirectOnly)
                .unwrap();
    }
    /// 第一台服务器的槽位目录（ADR-0042）。
    pub fn data_dir(&self) -> PathBuf {
        self._dir.path().join("servers/1")
    }

    pub async fn wait_onboarding(
        &self,
        mut done: impl FnMut(&OnboardingViewModel) -> bool,
    ) -> OnboardingViewModel {
        let deadline = Instant::now() + WAIT;
        loop {
            let view = self.app.onboarding();
            if done(&view) {
                return view;
            }
            assert!(Instant::now() < deadline, "{view:?}");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    pub fn device(&self) -> EndpointId {
        self.app.home().device_id.parse().unwrap()
    }

    pub fn connect(&self) {
        self.connect_to(self.home_ticket.clone());
    }

    pub fn connect_to(&self, ticket: String) {
        self.app
            .dispatch(Action::Home {
                action: wonton_protocol::HomeAction::ConnectHome { ticket },
            })
            .unwrap();
    }

    #[allow(dead_code)]
    pub fn unreachable_ticket(&self) -> String {
        ticket(
            self._home.server.endpoint().id(),
            SocketAddr::from((Ipv4Addr::LOCALHOST, 9)),
        )
    }

    /// 关掉服务端，模拟断网：客户端那边会看到连接断开。
    pub async fn drop_home(&self) {
        self._home.server.endpoint().close().await;
    }

    pub async fn connected(&self) {
        self.connected_within(WAIT).await;
    }

    /// 同 [`Self::connected`]，兜底预算由调用方按被测路径推导。
    pub async fn connected_within(&self, budget: Duration) {
        self.connect();
        let home = self.app.subscribe_home();
        let waiting = async {
            while let Some(view) = home.next().await {
                if view.status == HomeStatus::Connected && view.proxy.is_some() {
                    return;
                }
            }
            panic!("连接订阅提前结束");
        };
        // 等事件，不每 20ms 轮询：忙起来轮询自己也抢 CPU；超时只兜底，报出当时的状态。
        tokio::time::timeout(budget, waiting)
            .await
            .unwrap_or_else(|_| panic!("等待超时，当前：{:?}", self.app.home()));
    }

    pub async fn wait_for(
        &self,
        mut done: impl FnMut(&PluginsViewModel) -> bool,
    ) -> PluginsViewModel {
        let plugins = self.app.subscribe_plugins();
        let waiting = async {
            while let Some(view) = plugins.next().await {
                if done(&view) {
                    return view;
                }
            }
            panic!("订阅提前结束");
        };
        // 等事件，不每 20ms 轮询整份 ViewModel；超时只兜底，报出当时的状态。
        // 闭包只许读这份 ViewModel：读外部状态的只有等 DELETE 落到核心那一条，
        // 而 DELETE 一定先于随后的任务状态发布，事件等得到。
        tokio::time::timeout(WAIT, waiting)
            .await
            .unwrap_or_else(|_| panic!("等待超时，当前：{:?}", self.app.plugins()))
    }

    pub async fn task(
        &self,
        id: &str,
        mut done: impl FnMut(&PluginTaskState) -> bool,
    ) -> PluginTaskState {
        let view = self
            .wait_for(|view| {
                view.tasks
                    .iter()
                    .any(|task| task.id == id && done(&task.state))
            })
            .await;
        view.tasks
            .into_iter()
            .find(|task| task.id == id)
            .unwrap()
            .state
    }

    /// 同 [`Self::wait_for`]，兜底预算由调用方按被测路径推导：繁忙时一次请求实测会停滞数秒，
    /// 固定的 20 秒会成为绑定约束而不是兜底。
    pub async fn wait_for_within(
        &self,
        budget: Duration,
        mut done: impl FnMut(&PluginsViewModel) -> bool,
    ) -> PluginsViewModel {
        let plugins = self.app.subscribe_plugins();
        let waiting = async {
            while let Some(view) = plugins.next().await {
                if done(&view) {
                    return view;
                }
            }
            panic!("订阅提前结束");
        };
        tokio::time::timeout(budget, waiting)
            .await
            .unwrap_or_else(|_| panic!("等待超时，当前：{:?}", self.app.plugins()))
    }

    /// 同 [`Self::task`]，兜底预算见 [`Self::wait_for_within`]。
    pub async fn task_within(
        &self,
        id: &str,
        budget: Duration,
        mut done: impl FnMut(&PluginTaskState) -> bool,
    ) -> PluginTaskState {
        let view = self
            .wait_for_within(budget, |view| {
                view.tasks
                    .iter()
                    .any(|task| task.id == id && done(&task.state))
            })
            .await;
        view.tasks
            .into_iter()
            .find(|task| task.id == id)
            .unwrap()
            .state
    }
}
