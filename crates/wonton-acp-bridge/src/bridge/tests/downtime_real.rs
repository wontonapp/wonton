//! 真服务器上的停机积压：Tuwunel 起容器，桥的 Matrix 会话用真 SQLite 状态库分两段跑，
//! 中间停机发的那条要补处理，重启前处理过的不再重跑。

use std::time::Duration;

use wonton_matrix::testkit::{TestServer, wait_for_room};

use super::*;

#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn a_message_sent_while_down_is_processed_after_a_real_restart() {
    let dir = tempfile::tempdir().unwrap();
    let server = TestServer::start().await;
    // 主人建两人房间拉上 Bot：成员数 2，桥按私聊处理。
    let owner = server.login("owner", "主人").await;
    owner.start_sync().await.expect("主人同步");
    let bot_setup = server.login("bot", "桥").await;
    bot_setup.start_sync().await.expect("Bot 同步");
    let room_id = owner
        .create_group_room("桥", bot_setup.user_id())
        .await
        .expect("建房间");
    bot_setup.join_room(&room_id).await.expect("Bot 进房");
    let owner_id = owner.user_id().to_owned();
    let bot_token = bot_setup.tokens().expect("Bot 令牌").0;
    let owner_room = open_room(&owner, &room_id).await;
    // setup 会话到此为止：桥要用同一个设备令牌、同一个状态库登录。
    drop(bot_setup);

    let agent = FakeAgent::start(Scenario::Manual).await;
    let core = FakeCore::start().await;
    let config = Config {
        homeserver: server.url.clone(),
        access_token: bot_token,
        allowed_users: vec![owner_id.clone()],
        owner: owner_id,
        home_room: room_id,
        matrix_store: dir.path().join("matrix"),
        bridge_data: dir.path().join("bridge"),
        socket: agent.socket.clone(),
        workspace: "/workspace".into(),
        mcp_url: core.url(),
        mode: "plan".into(),
        topic_mode: None,
        health: crate::health::Files::in_dir(dir.path()),
        limits: Limits {
            max_input_bytes: 4096,
            max_output_bytes: 4096,
            max_queue: 8,
            turn_timeout: Duration::from_secs(20),
            idle_timeout: Duration::from_secs(600),
            idle_tick: Duration::from_millis(100),
            topic_idle: Duration::from_secs(1800),
            undecryptable_after: Duration::from_secs(60),
            undecryptable_every: Duration::from_secs(600),
        },
    };

    // 第一段：在线。主人的话照常处理。
    let (session, task) = spawn_bridge(&config).await;
    owner_room
        .send_text("在线时的话")
        .await
        .expect("主人发消息");
    wait_for_prompt(&agent, "在线时的话").await;
    task.abort();
    drop(task);
    // 释放 SQLite 状态库，第二段才能用同一个库登录。
    drop(session);
    tokio::time::sleep(Duration::from_millis(500)).await;

    // 停机期间主人的话。
    owner_room
        .send_text("停机期间的话")
        .await
        .expect("主人发消息");

    // 第二段：桥起（重新登录同一个状态库），停机期间的那条要补。
    let (_session, task) = spawn_bridge(&config).await;
    wait_for_prompt(&agent, "停机期间的话").await;
    // 在线时的那条整个测试里只发给过 Agent 一次，没有重跑。
    let repeats = agent
        .requests()
        .await
        .iter()
        .filter(|request| {
            request["method"] == "session/prompt"
                && request["params"]["prompt"][0]["text"] == "在线时的话"
        })
        .count();
    assert_eq!(repeats, 1, "重启前的消息不该重跑");
    task.abort();
    agent.finish().await;
    core.finish();
}

async fn open_room(session: &MatrixSession, room_id: &wonton_matrix::RoomId) -> ChatRoom {
    let room_id = room_id.to_owned();
    wait_for_room(session, "房间同步进来", &room_id, |row| {
        row.membership == wonton_matrix::Membership::Joined
    })
    .await;
    session.watch_room(&room_id).await.expect("开房间")
}

/// 和进程重启一样：按配置重新登录（同一个 SQLite 状态库）、起同步、开房间、连 Agent、跑主循环。
async fn spawn_bridge(config: &Config) -> (MatrixSession, JoinHandle<BridgeResult<()>>) {
    let session = MatrixSession::login_with_access_token(
        &SessionConfig {
            homeserver_url: config.homeserver.clone(),
            device_name: "Wonton ACP bridge".into(),
            store: StoreConfig::Sqlite {
                path: config.matrix_store.clone(),
                passphrase: None,
            },
            proxy: None,
        },
        &config.access_token,
    )
    .await
    .expect("桥登录");
    session.start_sync().await.expect("桥同步");
    let room = Arc::new(
        session
            .watch_room(&config.home_room)
            .await
            .expect("桥开房间"),
    );
    let acp = Arc::new(
        AcpClient::connect(
            &config.socket,
            crate::permission::PermissionBroker::new(&config.mcp_url).unwrap(),
        )
        .await
        .expect("桥连 Agent"),
    );
    let sessions = Arc::new(
        SessionStore::open(config.bridge_data.join("sessions.json"))
            .await
            .unwrap(),
    );
    let last_seen = Arc::new(
        LastSeen::open(config.bridge_data.join("last_seen.json"))
            .await
            .unwrap(),
    );
    let login = Arc::new(LoginService {
        flow: tokio::sync::Mutex::new(LoginFlow::new().0),
        direct: None,
        bot: session.user_id().to_owned(),
        acp: acp.clone(),
        socket: config.socket.clone(),
    });
    let bridge = Bridge {
        config: config.clone(),
        acp,
        sessions,
        last_seen,
        bot: session.user_id().to_owned(),
        locales: Arc::new(Locales::new(Origin::Fixed, session.user_id().to_owned())),
        chains: crate::bridge::chain::Client::new(&config.mcp_url, &config.access_token).unwrap(),
        pending_asks: permissions::PendingAsks::default(),
    };
    let task = tokio::spawn(run_room(bridge, room, false, None, login));
    (session, task)
}

/// 等到 Agent 手上出现这句话；真服务器起步几秒，5 秒的 wait_for 不够。
async fn wait_for_prompt(agent: &FakeAgent, text: &str) {
    for _ in 0..120 {
        if agent
            .prompts()
            .await
            .iter()
            .any(|(_, prompt)| prompt == text)
        {
            return;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    panic!("60 秒内没有等到 Agent 收到「{text}」");
}
