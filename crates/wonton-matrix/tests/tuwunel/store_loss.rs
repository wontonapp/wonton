//! Bot 的 crypto store 丢了（数据卷被删、容器重建没挂卷）或坏了之后会发生什么。
//!
//! Bot 账号没有开密钥备份（恢复密钥只有主人账号有），store 是设备密钥和房间密钥的唯一一份。实测
//! （Tuwunel v1.9，2026-11-11）：同一台设备带着空 store 重启后，服务器接受它重新上传的设备密钥，
//! 但它再也要不回房间密钥——别的设备跟它建 Olm 会话用的还是旧身份签的一次性密钥，建不起来就把
//! 房间密钥扣下（`m.room_key.withheld`，`m.no_olm`），已经认识这台设备的客户端还会拒绝它的新身份
//! （`SigningKeyChanged`）。于是它解不开任何人发的加密消息；重装插件换一台新设备才恢复。store 损坏
//! 则恢复会话直接失败，如实报错。

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use matrix_sdk::ruma::{OwnedDeviceId, OwnedRoomId, OwnedUserId};
use wonton_matrix::{Error, MatrixSession, MessageBody, SavedSession};

use crate::{
    chat,
    support::{TempStore, TestServer, eventually, unique},
};

/// 跑完第一段（互发过一条加密消息）的现场：继续用的主人会话、要丢掉的 store、
/// 登录状态、私聊房间和两边的用户、设备 ID。
struct Exchange {
    store: TempStore,
    saved: SavedSession,
    /// 主人的会话，保持同步着。
    owner: MatrixSession,
    room_id: OwnedRoomId,
    bot_user: OwnedUserId,
    bot_device: OwnedDeviceId,
    owner_user: OwnedUserId,
}

/// Bot 用持久存储登录，和主人建加密私聊，主人发一条、Bot 解开；然后 Bot 下线。
async fn exchanging(server: &TestServer) -> Exchange {
    let bot_local = unique("bot");
    let owner_local = unique("owner");
    let store = TempStore::create();
    let bot = server
        .login_with_store(&bot_local, "回声", store.config())
        .await;
    let saved = bot.saved().expect("登录状态");
    let bot_device = bot.device_id().to_owned();
    let bot_user = bot.user_id().to_owned();
    bot.start_sync().await.expect("Bot 同步");
    let owner = server.login(&owner_local, "主人").await;
    let owner_user = owner.user_id().to_owned();
    owner.start_sync().await.expect("主人同步");
    let room_id = chat::direct_chat(&owner, &bot, true).await;
    let owner_room = owner.open_room(&room_id).await.expect("主人打开私聊");
    owner_room.send_text("第一条").await.expect("主人发消息");
    let bot_room = bot.open_room(&room_id).await.expect("Bot 打开私聊");
    eventually(
        "Bot 解开第一条",
        bot_room.message_updates().await,
        |messages| {
            messages
                .iter()
                .any(|message| message.body == MessageBody::Text("第一条".into()))
                .then_some(())
        },
    )
    .await;
    bot.stop_sync().await;
    drop(bot);
    Exchange {
        store,
        saved,
        owner,
        room_id,
        bot_user,
        bot_device,
        owner_user,
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("系统时间")
        .as_millis() as u64
}

/// 等主人（任何设备）在 `before` 之后发的一条出现，返回 Bot 眼里它解没解开。
async fn owner_message_seen(
    what: &str,
    room: &wonton_matrix::ChatRoom,
    owner_user: &wonton_matrix::UserId,
    before: u64,
) -> bool {
    eventually(what, room.message_updates().await, |messages| {
        messages
            .iter()
            .find(|message| message.sender == owner_user && message.timestamp_ms >= before)
            .map(|message| matches!(message.body, MessageBody::Undecryptable))
    })
    .await
}

#[tokio::test]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn a_bot_with_a_lost_store_stays_deaf_until_reinstalled() {
    let server = TestServer::start().await;
    let exchange = exchanging(&server).await;
    let Exchange {
        store,
        saved,
        owner,
        room_id,
        bot_user,
        bot_device,
        owner_user,
    } = exchange;

    // 数据卷没了：store 目录整个删掉，同一个令牌、同一台设备重启（桥 login_with_access_token 拿到
    // 同一个设备 ID，restore_session 建出全新的 Olm 账号）。
    store.wipe();
    let restarted = MatrixSession::restore(&server.config("回声", store.config()), saved)
        .await
        .expect("空 store 恢复会话");
    assert_eq!(
        restarted.device_id(),
        bot_device,
        "同一个令牌恢复出来的还是同一台设备"
    );
    restarted.start_sync().await.expect("重启后同步");
    let room = restarted.open_room(&room_id).await.expect("重启后打开私聊");
    room.paginate_backwards(20).await.expect("向前翻页");

    // 历史解不开（没有备份可取）。
    eventually(
        "重启后历史解不开",
        room.message_updates().await,
        |messages| {
            messages
                .iter()
                .any(|message| matches!(message.body, MessageBody::Undecryptable))
                .then_some(())
        },
    )
    .await;

    // 它自己还能发：新出站会话，主人解得开。
    room.send_text("重启后的回声").await.expect("Bot 发消息");
    let owner_room = owner.open_room(&room_id).await.expect("主人打开私聊");
    eventually(
        "主人解开重启后的回声",
        owner_room.message_updates().await,
        |messages| {
            messages
                .iter()
                .any(|message| message.body == MessageBody::Text("重启后的回声".into()))
                .then_some(())
        },
    )
    .await;

    // 但主人再发的它解不开：旧出站会话的房间密钥只在丢掉的 store 里，不会重发给"同一台设备"。
    let before = now_ms();
    owner_room.send_text("第二条").await.expect("主人再发");
    assert!(
        owner_message_seen("Bot 解不开主人的新消息", &room, &owner_user, before).await,
        "Bot 应该解不开主人的新消息"
    );

    // 换一台全新的主人设备也一样：跟这台设备建 Olm 会话用的还是旧身份签的一次性密钥，房间密钥被扣。
    let probe = server.login(owner_user.localpart(), "探针").await;
    probe.start_sync().await.expect("探针同步");
    tokio::time::sleep(Duration::from_secs(3)).await;
    probe.wait_for_server_device(&bot_user, &bot_device).await;
    let before = now_ms();
    probe
        .open_room(&room_id)
        .await
        .expect("探针打开私聊")
        .send_text("探针发的")
        .await
        .expect("探针发消息");
    assert!(
        owner_message_seen("Bot 解不开全新设备的消息", &room, &owner_user, before).await,
        "Bot 应该解不开全新设备的消息"
    );
    probe.stop_sync().await;
    drop(probe);
    restarted.stop_sync().await;
    drop(restarted);

    // 重装插件：核心吊销旧设备、代 Bot 登录一台新设备，旧 store 作废——收发恢复，历史仍然解不开。
    let fresh = server.login(&localpart_of(&bot_user), "回声").await;
    assert_ne!(fresh.device_id(), bot_device, "重装后应该是一台新设备");
    fresh.start_sync().await.expect("重装后同步");
    let fresh_room = fresh.open_room(&room_id).await.expect("重装后打开私聊");
    owner_room.send_text("第三条").await.expect("主人再发");
    eventually(
        "重装后的 Bot 解开主人的新消息",
        fresh_room.message_updates().await,
        |messages| {
            messages
                .iter()
                .any(|message| message.body == MessageBody::Text("第三条".into()))
                .then_some(())
        },
    )
    .await;
}

/// `@bot-xxxx:域名` 里的 localpart。
fn localpart_of(user: &wonton_matrix::UserId) -> String {
    user.localpart().to_owned()
}

#[tokio::test]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn a_bot_with_a_corrupted_store_fails_to_restore() {
    let server = TestServer::start().await;
    let bot_local = unique("bot");

    let store = TempStore::create();
    let bot = server
        .login_with_store(&bot_local, "回声", store.config())
        .await;
    let saved = bot.saved().expect("登录状态");
    bot.stop_sync().await;
    drop(bot);

    // store 坏了（SQLite 损坏）：恢复会话直接失败，不伪装成新开始。
    store.corrupt();
    let error = MatrixSession::restore(&server.config("回声", store.config()), saved)
        .await
        .err()
        .expect("损坏的 store 应该让恢复失败");
    assert!(
        matches!(error, Error::Upstream(_) | Error::Transport(_)),
        "{error}"
    );
}
