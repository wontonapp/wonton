//! 桥接层的行为测试：给定 Matrix 消息和 ACP 消息序列，断言 Matrix 侧副作用和发给 Agent
//! 的请求。房间是内存替身，ACP 是 Manual 场景的假 Agent，确认接口是假卡片。

mod cards;
mod downtime;
mod downtime_real;
mod limits;
mod restore;
mod run_status;
mod tasks;
mod topics;

use std::sync::Arc;

use tempfile::TempDir;
use tokio::task::JoinHandle;
use wonton_matrix::{Message, OwnedUserId};

use super::test_support::{FakeCards, FakeRoom, owner_message, owner_message_at, wait_until};
use super::*;
use crate::acp::AcpClient;
use crate::acp::test_support::{FakeAgent, Scenario, bounded};
use crate::config::{Config, Limits};
use crate::locale::{Locales, Origin};
use crate::login::LoginFlow;
use crate::state::SessionStore;

const OWNER: &str = "@owner:example.org";
const BOT: &str = "@bot:example.org";

struct Harness {
    _dir: TempDir,
    room: Arc<FakeRoom>,
    agent: FakeAgent,
    cards: FakeCards,
    config: Config,
    acp: Arc<AcpClient>,
    store: Arc<SessionStore>,
    task: Option<JoinHandle<BridgeResult<()>>>,
}

impl Harness {
    async fn start() -> Self {
        Self::with(|_| {}).await
    }

    /// 起一套运行环境，首屏快照里带上 `backlog`（重启后停机期间看得见的消息）。
    async fn start_with(backlog: Vec<Message>) -> Self {
        Self::build(|_| {}, backlog).await
    }

    /// 起一套运行环境；`tune` 在启动前改配置（调小空闲、时限等）。
    async fn with(tune: impl FnOnce(&mut Config)) -> Self {
        Self::build(tune, Vec::new()).await
    }

    async fn build(tune: impl FnOnce(&mut Config), backlog: Vec<Message>) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let agent = FakeAgent::start(Scenario::Manual).await;
        let cards = FakeCards::start().await;
        let mut config = Config {
            homeserver: "http://example.org".into(),
            access_token: "token".into(),
            owner: OWNER.parse::<OwnedUserId>().unwrap(),
            allowed_users: vec![OWNER.parse::<OwnedUserId>().unwrap()],
            home_room: "!room:example.org".parse().unwrap(),
            matrix_store: dir.path().join("matrix"),
            bridge_data: dir.path().join("bridge"),
            socket: agent.socket.clone(),
            workspace: "/workspace".into(),
            mcp_url: cards.url(),
            mode: "plan".into(),
            topic_mode: Some("workspace-write".into()),
            limits: Limits {
                max_input_bytes: 4096,
                max_output_bytes: 4096,
                max_queue: 8,
                turn_timeout: std::time::Duration::from_secs(20),
                idle_timeout: std::time::Duration::from_secs(3600),
                idle_tick: std::time::Duration::from_millis(20),
                topic_idle: std::time::Duration::from_secs(1800),
            },
        };
        tune(&mut config);
        let acp = Arc::new(
            bounded(
                AcpClient::connect(
                    &config.socket,
                    crate::permission::PermissionBroker::new(&config.mcp_url).unwrap(),
                ),
                "connect",
            )
            .await
            .unwrap(),
        );
        let store = Arc::new(
            SessionStore::open(config.bridge_data.join("sessions.json"))
                .await
                .unwrap(),
        );
        let room = Arc::new(FakeRoom::with_snapshot(config.home_room.as_str(), backlog));
        let mut harness = Self {
            _dir: dir,
            room,
            agent,
            cards,
            config,
            acp,
            store,
            task: None,
        };
        harness.spawn_loop().await;
        harness
    }

    async fn spawn_loop(&mut self) {
        let bridge = Bridge {
            config: self.config.clone(),
            acp: self.acp.clone(),
            sessions: self.store.clone(),
            // 每次起循环重新从盘上读：和进程重启一样，已读位置必须落过盘才作数。
            last_seen: Arc::new(
                LastSeen::open(self.config.bridge_data.join("last_seen.json"))
                    .await
                    .unwrap(),
            ),
            bot: BOT.parse().unwrap(),
            locales: Arc::new(Locales::new(Origin::Fixed, BOT.parse().unwrap())),
        };
        let room = self.room.clone();
        let login = Arc::new(LoginService {
            flow: tokio::sync::Mutex::new(LoginFlow::new().0),
            direct: None,
            bot: BOT.parse().unwrap(),
            acp: self.acp.clone(),
            socket: self.config.socket.clone(),
        });
        self.task = Some(tokio::spawn(run_room(bridge, room, false, None, login)));
    }

    /// 重起循环：换一个新房间，保存的会话映射和数据目录不变；`backlog` 是新房间首屏
    /// 快照里的消息（和真时间线一样，重启后停机期间的都看得见）。
    async fn restart_with(&mut self, backlog: Vec<Message>) {
        self.room.close();
        if let Some(task) = self.task.take() {
            task.abort();
        }
        self.room = Arc::new(FakeRoom::with_snapshot(
            self.config.home_room.as_str(),
            backlog,
        ));
        self.spawn_loop().await;
    }

    async fn restart(&mut self) {
        self.restart_with(Vec::new()).await;
    }

    fn send(&self, event_id: &str, text: &str) {
        self.room.push(vec![owner_message(event_id, text, None)]);
    }

    /// 同 [`Self::send`]，时间戳自定：积压测试要拿它跟已读位置比先后。
    fn send_at(&self, event_id: &str, timestamp_ms: u64, text: &str) {
        self.room
            .push(vec![owner_message_at(event_id, text, None, timestamp_ms)]);
    }

    fn send_in_thread(&self, event_id: &str, root: &str, text: &str) {
        self.room
            .push(vec![owner_message(event_id, text, Some(root))]);
    }

    /// 现在挂在 Agent 手上、还没应答的轮次：（会话，原文）。
    async fn prompts(&self) -> Vec<(String, String)> {
        self.agent.prompts().await
    }

    /// 等到 Agent 手上有 `count` 个挂起的轮次。
    async fn wait_prompts(&self, count: usize) -> Vec<(String, String)> {
        let agent = &self.agent;
        wait_until(|| agent.pending_count() >= count).await;
        agent.prompts().await
    }

    /// 回掉 `session` 上挂起的那一轮。
    async fn respond(&self, session: &str, text: &str) {
        let index = self
            .prompts()
            .await
            .iter()
            .position(|(pending, _)| pending == session)
            .expect("这个会话上没有挂起的轮次");
        self.agent.respond_prompt(index, text).await;
    }

    async fn finish(self) {
        self.room.close();
        if let Some(task) = self.task {
            task.abort();
        }
        self.agent.finish().await;
        self.cards.finish();
    }
}
