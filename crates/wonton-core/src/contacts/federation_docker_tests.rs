//! 真实 Tuwunel（v1.9.4）上跨服务器的 Bot 联系人走通（ADR-0050）：两台“核心”各带真的 Matrix 连接和
//! 收件循环，联系人请求、同意、删除都经核心账号之间的房间过真联邦。转发组件和 Tuwunel 容器的搭建
//! 复用 `matrix::federation_config::proxy_tests` 的那套（那边的测试证过联邦本身通）。

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::{Duration, Instant};

use wonton_protocol::{BotRef, ContactKind, ContactRequest, ContactState};

use super::{
    Contacts, exchange,
    test_support::{FakeBots, FakePeers, FakePush},
};
use crate::config::MatrixConfig;
use crate::http::HttpClient;
use crate::matrix::Matrix;
use crate::matrix::federation_config::proxy_tests::{Core, Pairings, Stack, until};
use crate::matrix::federation_config::tests::{create_room, invite, join};
use tracing_subscriber::EnvFilter;

/// 两台“核心”和两套安装：配对、路由、名单热加载都就绪，联系人只管发事件。
/// Core 必须活着：转发组件一 drop 就 stop_all 断掉隧道，后面的联系人阶段全发不出去（就是字段
/// “没读”也要留在这里）。
struct TwoSides {
    stack_a: Stack,
    stack_b: Stack,
    dir_a: tempfile::TempDir,
    dir_b: tempfile::TempDir,
    #[allow(dead_code)]
    core_a: Core,
    #[allow(dead_code)]
    core_b: Core,
    _routers: [iroh::protocol::Router; 2],
}

async fn two_sides() -> TwoSides {
    let pairings: Pairings = Arc::default();
    let (relays, _, _relay) = iroh::test_utils::run_relay_server().await.unwrap();
    let (core_a, core_b) = tokio::join!(
        Core::start(&pairings, &relays),
        Core::start(&pairings, &relays)
    );
    let dir_a = tempfile::tempdir().unwrap();
    let dir_b = tempfile::tempdir().unwrap();
    let (stack_a, stack_b) = std::thread::scope(|scope| {
        let a = scope.spawn(|| Stack::start(dir_a.path(), core_a.proxy_port));
        let b = scope.spawn(|| Stack::start(dir_b.path(), core_b.proxy_port));
        (a.join().unwrap(), b.join().unwrap())
    });
    pairings.lock().unwrap().extend([
        (
            stack_a.server_name.clone(),
            (core_a.id(), core_a.endpoint.addr()),
        ),
        (
            stack_b.server_name.clone(),
            (core_b.id(), core_b.endpoint.addr()),
        ),
    ]);
    let routers = [
        core_a.accept_from(&core_b, stack_a.federation_port),
        core_b.accept_from(&core_a, stack_b.federation_port),
    ];
    core_a
        .paired
        .send_replace(BTreeSet::from([stack_b.server_name.clone()]));
    core_b
        .paired
        .send_replace(BTreeSet::from([stack_a.server_name.clone()]));

    // 名单热加载要 Tuwunel 起好才开始收信号。
    wait_ready(&stack_a.client_address()).await;
    wait_ready(&stack_b.client_address()).await;
    stack_a.allow(&stack_b.server_name);
    stack_b.allow(&stack_a.server_name);
    // 配对后转发组件马上探活；对端的联邦 TLS 监听起得慢时第一轮探活失败会进退避。
    // 等两边都探活成功再动手，不然出站请求全在退避里被立刻拒绝。
    until("转发探活两边都是 Connected", || {
        let (core_a, core_b, name_a, name_b) = (
            &core_a,
            &core_b,
            stack_a.server_name.clone(),
            stack_b.server_name.clone(),
        );
        async move {
            match (core_a.peer_status(&name_b), core_b.peer_status(&name_a)) {
                (
                    Some(wonton_tunnel::federation::PeerStatus::Connected),
                    Some(wonton_tunnel::federation::PeerStatus::Connected),
                ) => Ok(()),
                (a, b) => Err(format!("还没通：{a:?} / {b:?}")),
            }
        }
    })
    .await
    .unwrap();
    // 联邦预热：一次真人的跨机邀请/加入。Tuwunel 会把第一次没送出去的联邦请求缓存成
    // "对方不可达"；生产里两台服务器早有真人往来，测试照真实时序先把这条路走通。
    let (server_a, server_b) = (stack_a.server(), stack_b.server());
    let alice = stack_a.register("alice").await;
    let bob = stack_b.register("bob").await;
    let warm_room = create_room(&server_a, &alice).await;
    until("联邦预热：跨机邀请和加入", || {
        let (server_a, server_b, alice, bob, warm_room) =
            (&server_a, &server_b, &alice, &bob, warm_room.clone());
        async move {
            invite(server_a, alice, &warm_room, &bob.user_id).await?;
            join(server_b, &bob.token, &warm_room).await
        }
    })
    .await
    .unwrap();
    TwoSides {
        stack_a,
        stack_b,
        dir_a,
        dir_b,
        core_a,
        core_b,
        _routers: routers,
    }
}

/// A 发出跨机请求。意图先落盘（outbox），接口立刻返回；真正的送达由下面的 until 等 B 的推送和行。
async fn request_until_sent(side_a: &Side, bot_a: &BotRef, user_b: &str) {
    until("A 发出联系人请求", || {
        let (side_a, bot_a, user_b) = (side_a, bot_a.clone(), user_b.to_owned());
        async move {
            side_a
                .contacts
                .request(
                    &bot_a,
                    &ContactRequest {
                        user_id: user_b,
                        kind: ContactKind::Bot,
                    },
                )
                .await
                .map(|_| ())
                .map_err(|e| format!("还没发出去：{e:?}"))
        }
    })
    .await
    .unwrap();
}

#[test]
#[ignore = "需要 Docker，跑一遍约两分钟"]
fn contacts_exchange_over_real_federation() {
    // 核心的日志（房间补建、收件循环）只在初始化了订阅者时才看得到。
    let _ = tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_new("info").unwrap())
        .with_test_writer()
        .try_init();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let two = two_sides().await;
        let (stack_a, stack_b) = (&two.stack_a, &two.stack_b);
        let side_a = start_side(&two.dir_a, stack_a, &stack_b.server_name).await;
        let side_b = start_side(&two.dir_b, stack_b, &stack_a.server_name).await;
        let bot_a = BotRef {
            plugin_id: "com.example.alpha".into(),
            bot: "helper".into(),
        };
        let bot_b = bot_a.clone();
        let user_a = format!("@helper-a:{}", stack_a.server_name);
        let user_b = format!("@helper-b:{}", stack_b.server_name);
        side_a
            .bots
            .add(&user_a, &bot_a, "本机的福", "主人甲", "owner-a");
        side_b
            .bots
            .add(&user_b, &bot_b, "那边的福", "主人乙", "owner-b");

        // A 的 Bot 请 B 的 Bot：请求过真联邦到 B，B 的归属人收到推送。
        request_until_sent(&side_a, &bot_a, &user_b).await;
        until("B 收到联系人请求", || {
            let (side_b, user_a) = (&side_b, &user_a);
            async move {
                match side_b.push.requests().first() {
                    Some((keeper, request)) => {
                        assert_eq!(keeper.as_deref(), Some("owner-b"));
                        assert_eq!(request.user_id, *user_a);
                        Ok(())
                    }
                    None => Err("还没有推送".to_owned()),
                }
            }
        })
        .await
        .unwrap();
        until("B 的行是 Incoming", || {
            let (side_b, bot_b, user_a) = (&side_b, &bot_b, &user_a);
            async move {
                match state_of(side_b, bot_b, user_a) {
                    Some(ContactState::Incoming) => Ok(()),
                    other => Err(format!("还是 {other:?}")),
                }
            }
        })
        .await
        .unwrap();
        assert_eq!(
            state_of(&side_a, &bot_a, &user_b),
            Some(ContactState::Pending)
        );

        // B 同意：结果送回 A，两边都 Active。
        side_b.contacts.approve(&bot_b, &user_a).await.unwrap();
        until("A 的行落成 Active", || {
            let (side_a, bot_a, user_b) = (&side_a, &bot_a, &user_b);
            async move {
                match state_of(side_a, bot_a, user_b) {
                    Some(ContactState::Active) => Ok(()),
                    other => Err(format!("还是 {other:?}")),
                }
            }
        })
        .await
        .unwrap();

        // A 删掉：Remove 送到 B，两边的行都没了。
        side_a.contacts.delete(&bot_a, &user_b).await.unwrap();
        until("B 的行被删掉", || {
            let (side_b, bot_b, user_a) = (&side_b, &bot_b, &user_a);
            async move {
                match state_of(side_b, bot_b, user_a) {
                    None => Ok(()),
                    other => Err(format!("还有 {other:?}")),
                }
            }
        })
        .await
        .unwrap();
        assert!(state_of(&side_a, &bot_a, &user_b).is_none());
    });
}

/// 一侧的联系人表和它的替身。
struct Side {
    contacts: Arc<Contacts>,
    bots: Arc<FakeBots>,
    push: Arc<FakePush>,
}

fn state_of(side: &Side, bot: &BotRef, user: &str) -> Option<ContactState> {
    side.contacts.stored_row(bot, user).map(|row| row.state)
}

/// 一侧：Matrix 连接、联系人表、收件循环。配对名单的替身直接登记对方。
async fn start_side(dir: &tempfile::TempDir, stack: &Stack, peer_server: &str) -> Side {
    let config = MatrixConfig {
        url: stack.client_address(),
        server_name: stack.server_name.clone(),
        jwt_key: std::fs::read_to_string(dir.path().join("secrets/matrix-jwt.pem")).unwrap(),
        registration_secret: stack.registration_secret.clone(),
    };
    let matrix = Arc::new(
        Matrix::connect(&config, dir.path(), HttpClient::new())
            .await
            .unwrap(),
    );
    let peers = Arc::new(FakePeers::default());
    peers.pair(peer_server);
    let bots = Arc::new(FakeBots::default());
    let push = Arc::new(FakePush::default());
    let contacts = Contacts::open(dir.path().join("contacts.json"), peers)
        .unwrap()
        .with_matrix(matrix)
        .with_bots(bots.clone())
        .with_push(push.clone());
    exchange::spawn(&contacts);
    Side {
        contacts,
        bots,
        push,
    }
}

/// 等 Tuwunel 的客户端接口就绪（`federation_config::tests::wait_ready` 不出那个模块，这里照抄一份）。
async fn wait_ready(base: &str) {
    let http = HttpClient::new();
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        if http
            .get_status(
                &format!("{base}/_matrix/client/versions"),
                Duration::from_secs(5),
            )
            .await
            .is_ok()
        {
            return;
        }
        assert!(Instant::now() < deadline, "Tuwunel 一直没就绪：{base}");
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
}
