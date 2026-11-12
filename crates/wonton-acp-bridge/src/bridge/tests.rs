//! 桥接层的行为测试：给定 Matrix 消息和 ACP 消息序列，断言 Matrix 侧副作用和发给 Agent
//! 的请求。房间是内存替身，ACP 是 Manual 场景的假 Agent，确认接口是假卡片。

mod cards;
mod limits;
mod restore;
mod topics;

use std::sync::Arc;

use tempfile::TempDir;
use tokio::task::JoinHandle;
use wonton_matrix::OwnedUserId;

use super::test_support::{FakeCards, FakeRoom, owner_message, wait_until};
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

    /// 起一套运行环境；`tune` 在启动前改配置（调小空闲、时限等）。
    async fn with(tune: impl FnOnce(&mut Config)) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let agent = FakeAgent::start(Scenario::Manual).await;
        let cards = FakeCards::start().await;
        let mut config = Config {
            homeserver: "http://example.org".into(),
            access_token: "token".into(),
            owner: OWNER.parse::<OwnedUserId>().unwrap(),
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
        let room = Arc::new(FakeRoom::new(config.home_room.as_str()));
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
        harness.spawn_loop();
        harness
    }

    fn spawn_loop(&mut self) {
        let bridge = Bridge {
            config: self.config.clone(),
            acp: self.acp.clone(),
            sessions: self.store.clone(),
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

    /// 重起循环：换一个新房间，保存的会话映射和数据目录不变。
    fn restart(&mut self) {
        self.room.close();
        if let Some(task) = self.task.take() {
            task.abort();
        }
        self.room = Arc::new(FakeRoom::new(self.config.home_room.as_str()));
        self.spawn_loop();
    }

    fn send(&self, event_id: &str, text: &str) {
        self.room.push(vec![owner_message(event_id, text, None)]);
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
