//! 真实 Tuwunel 上的“长期解不开”行为：crypto store 丢了、同一台设备重启后，主人发消息解不开，
//! 桥过时限在房间里说一次“读不了”；启动时的历史解不开不触发；10 分钟内不再说第二遍。
//! 走的是真房间、真加密、真主人的时间线，桥的 ACP 和确认接口仍是替身。

use std::sync::Arc;
use std::time::Duration;

use futures_util::StreamExt;
use wonton_matrix::testkit::{TempStore, TestServer, unique};
use wonton_matrix::{
    MatrixSession, MessageBody, OwnedRoomId, OwnedUserId, SavedSession, SessionConfig, UserId,
};

use super::*;
use crate::acp::AcpClient;
use crate::acp::test_support::{FakeAgent, Scenario, bounded};
use crate::bridge::LoginService;
use crate::bridge::test_support::FakeCards;
use crate::config::Config;
use crate::locale::{Locales, Origin};
use crate::login::LoginFlow;
use crate::permission::PermissionBroker;
use crate::state::SessionStore;

/// 桥的“读不了”时限（测试里配短），扫的节奏是它的 1/4。
const AFTER: Duration = Duration::from_millis(600);
/// 再提醒的间隔：测试里配长，断言“只说一次”。
const EVERY: Duration = Duration::from_secs(600);

/// 主人和 Bot 建好加密私聊、互发过一条（Bot 解开了），然后 Bot 下线。
struct Setup {
    server: TestServer,
    owner: MatrixSession,
    owner_room: wonton_matrix::ChatRoom,
    room_id: OwnedRoomId,
    bot_user: OwnedUserId,
    store: TempStore,
    saved: SavedSession,
}

async fn setup() -> Setup {
    let server = TestServer::start().await;
    let bot_local = unique("bot");
    let owner_local = unique("owner");
    let store = TempStore::create();
    let bot = server
        .login_with_store(&bot_local, "回声", store.config())
        .await;
    let saved = bot.saved().expect("登录状态");
    let bot_user = bot.user_id().to_owned();
    bot.start_sync().await.expect("Bot 同步");
    let owner = server.login(&owner_local, "主人").await;
    owner.start_sync().await.expect("主人同步");
    let room_id = direct_chat(&owner, &bot).await;
    let owner_room = owner.open_room(&room_id).await.expect("主人打开私聊");
    owner_room.send_text("第一条").await.expect("主人发消息");
    let bot_room = bot.open_room(&room_id).await.expect("Bot 打开私聊");
    tokio::time::timeout(Duration::from_secs(20), async {
        bot_room
            .message_updates()
            .await
            .any(|messages| async move {
                messages
                    .iter()
                    .any(|message| message.body == MessageBody::Text("第一条".into()))
            })
            .await
    })
    .await
    .expect("Bot 解开第一条");
    bot.stop_sync().await;
    drop(bot);
    Setup {
        server,
        owner,
        owner_room,
        room_id,
        bot_user,
        store,
        saved,
    }
}

/// `alice` 建加密私聊邀请 `bob`，`bob` 看到邀请后加入（同 wonton-matrix 的 store_loss 测试）。
async fn direct_chat(owner: &MatrixSession, bot: &MatrixSession) -> OwnedRoomId {
    let room_id = owner
        .create_direct_chat(bot.user_id(), true)
        .await
        .expect("建私聊");
    for _ in 0..50 {
        if bot.join_room(&room_id).await.is_ok() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    owner
        .wait_for_server_device(bot.user_id(), bot.device_id())
        .await;
    room_id
}

fn is_notice(message: &wonton_matrix::Message, bot_user: &UserId) -> bool {
    message.sender == bot_user
        && matches!(&message.body, MessageBody::Text(text) if text.contains("读不了"))
}

/// 同一台设备带着空 store 重启（桥 `login_with_access_token` 的路径：whoami 同设备、
/// `reconcile_store` 不动空目录、`restore_session` 建出全新 Olm 账号），把桥的房间循环跑起来，
/// 返回要一直活到测试结束的 Bot 会话：房间流跟它共享同步，停了桥就什么都收不到。断言走
/// `Setup::owner_room`（主人的时间线）。
async fn restarted_bridge(setup: &Setup) -> MatrixSession {
    setup.store.wipe();
    let session_config = SessionConfig {
        homeserver_url: setup.server.url.clone(),
        device_name: "回声".into(),
        store: setup.store.config(),
        proxy: None,
    };
    let bot = MatrixSession::restore(&session_config, setup.saved.clone())
        .await
        .expect("空 store 恢复会话");
    assert_eq!(bot.device_id(), setup.saved.device_id(), "还是同一台设备");
    bot.start_sync().await.expect("重启后同步");
    let room = Arc::new(
        bot.watch_room(&setup.room_id)
            .await
            .expect("重启后打开私聊"),
    );

    let agent = FakeAgent::start(Scenario::Manual).await;
    let cards = FakeCards::start().await;
    let dir = tempfile::tempdir().unwrap();
    let owner = setup.owner.user_id().to_owned();
    let config = Config {
        homeserver: setup.server.url.clone(),
        access_token: "test-only".into(),
        owner: owner.clone(),
        allowed_users: vec![owner.clone()],
        home_room: setup.room_id.clone(),
        matrix_store: dir.path().join("matrix"),
        bridge_data: dir.path().join("bridge"),
        socket: agent.socket.clone(),
        workspace: "/workspace".into(),
        mcp_url: cards.url(),
        mode: "plan".into(),
        topic_mode: None,
        limits: crate::config::Limits {
            max_input_bytes: 4096,
            max_output_bytes: 4096,
            max_queue: 8,
            turn_timeout: Duration::from_secs(20),
            idle_timeout: Duration::from_secs(3600),
            idle_tick: Duration::from_millis(50),
            topic_idle: Duration::from_secs(1800),
            undecryptable_after: AFTER,
            undecryptable_every: EVERY,
        },
    };
    let acp = Arc::new(
        bounded(
            AcpClient::connect(
                &config.socket,
                PermissionBroker::new(&config.mcp_url).unwrap(),
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
    let bridge = Bridge {
        config: config.clone(),
        acp: acp.clone(),
        sessions: store,
        last_seen: Arc::new(
            LastSeen::open(config.bridge_data.join("last_seen.json"))
                .await
                .unwrap(),
        ),
        bot: setup.bot_user.clone(),
        locales: Arc::new(Locales::new(Origin::Fixed, setup.bot_user.clone())),
    };
    let login = Arc::new(LoginService {
        flow: tokio::sync::Mutex::new(LoginFlow::new().0),
        direct: None,
        bot: setup.bot_user.clone(),
        acp: acp.clone(),
        socket: config.socket.clone(),
    });
    tokio::spawn(run_room(bridge, room, false, None, login));
    bot
}

#[tokio::test]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn a_bot_that_lost_its_store_says_so_once() {
    let setup = setup().await;
    let _bot = restarted_bridge(&setup).await;
    let mut updates = setup.owner_room.message_updates().await;

    // 启动时的历史（第一条）解不开不算“收到解不开的消息”：这段时间桥不该说话。
    tokio::time::sleep(2 * AFTER + Duration::from_millis(500)).await;

    // 主人再发一条：解不开，过时限桥说一次。
    setup
        .owner_room
        .send_text("第二条")
        .await
        .expect("主人再发");
    tokio::time::timeout(Duration::from_secs(10), async {
        while let Some(messages) = updates.next().await {
            if messages
                .iter()
                .any(|message| is_notice(message, &setup.bot_user))
            {
                return;
            }
        }
    })
    .await
    .expect("桥应该在房间里说读不了");

    // 间隔内不再说第二遍。时间线快照会把同一批事件再发一遍：按事件 ID 去重，别把同一条
    // 提示数成两条。
    setup
        .owner_room
        .send_text("第三条")
        .await
        .expect("主人再发");
    let second = tokio::time::timeout(2 * AFTER + Duration::from_secs(2), async {
        let mut noticed = std::collections::HashSet::new();
        while let Some(messages) = updates.next().await {
            for message in messages {
                if is_notice(&message, &setup.bot_user)
                    && noticed.insert(message.event_id.clone().expect("提示有事件 ID"))
                    && noticed.len() > 1
                {
                    return;
                }
            }
        }
    })
    .await;
    assert!(second.is_err(), "10 分钟内只说一次");
    // 行为测试真写了宿主的 /tmp 标记，跑完清掉。
    crate::health::clear_state_for_test();
}
