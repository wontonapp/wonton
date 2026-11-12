//! 房间订阅按状态挂稳（#370）：列表行抖动不退订打开着的房间，同步重启后把订阅补挂回去。用模拟核心
//! 记下的同步请求体断言，不靠睡眠。

use std::time::Duration;

use wonton_client::{Action, RoomMembership};
use wonton_matrix::ChatRoom;

use super::confirmation::joined_direct_chat;
use super::stale_join::StaleJoin;
use super::support::{Home, OWNER, Phone, Timeline, WAIT, is_room_list};
use super::text;

/// 本地入口序号之后的房间列表请求，含首条无 pos 请求；取消前的旧请求可比替代请求晚到。
fn room_list_requests(home: &Home, after: u64) -> Vec<(Option<String>, Vec<String>)> {
    home.sync_requests()
        .into_iter()
        .filter(|request| {
            is_room_list(request.get("conn_id").unwrap_or(&serde_json::Value::Null))
                && request["test_sequence"].as_u64().unwrap() > after
        })
        .map(|request| {
            (
                request
                    .get("pos")
                    .and_then(|v| v.as_str())
                    .map(String::from),
                request
                    .get("room_subscriptions")
                    .and_then(|v| v.as_object())
                    .map(|subs| subs.keys().cloned().collect())
                    .unwrap_or_default(),
            )
        })
        .collect()
}

/// 轮询测试代理记下的请求，直到满足条件；超时保留完整请求序列。
async fn wait_for_sync<R>(what: &str, home: &Home, mut check: impl FnMut() -> Option<R>) -> R {
    let deadline = tokio::time::Instant::now() + WAIT;
    loop {
        if let Some(found) = check() {
            return found;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "{what}：{WAIT:?} 内没有等到；请求：{:#?}",
            home.sync_requests()
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// 让列表行错标成受邀再恢复：同步重启后的第一个应答带着加入事件，被改写成“受邀”（Tuwunel 补状态
/// 前的样子，见 [`super::stale_join`]），再重启一次用没改写的应答把状态补回来。两次重启都靠
/// `M_UNKNOWN_POS`；在飞的长轮询用另一个房间的一条消息结束，不动这个房间的时间线。
async fn flicker(phone: &Phone, home: &Home, poke_room: &ChatRoom, room_id: &str, label: &str) {
    home.report_join_as_invited(StaleJoin {
        room_id: room_id.to_owned(),
        user_id: OWNER.to_owned(),
    });
    home.expire_room_list_sync();
    poke_room
        .send_text("戳一下长轮询")
        .await
        .expect("戳一下同步");
    // 叫醒等退避的重启：回到前台本来就不等间隔（`MatrixSession::resume`），测试里逐次注入用得上。
    phone.dispatch(Action::AppForegrounded).unwrap();
    phone
        .wait_chat("行错标成受邀", |view| {
            view.rooms
                .iter()
                .find(|row| row.room_id == room_id && row.membership == RoomMembership::Invited)
                .map(|_| ())
        })
        .await;
    assert!(
        home.join_was_reported_as_invited(),
        "{label}：改写没有发生，行不是被改错标的"
    );
    // 错标已经看到：停止改写，下一次重启的应答照原样带回真实状态，行回到已加入。
    home.clear_join_report_as_invited();
    home.expire_room_list_sync();
    poke_room.send_text("再戳一下").await.expect("再戳一下同步");
    phone.dispatch(Action::AppForegrounded).unwrap();
    phone
        .wait_chat("行回到已加入", |view| {
            view.rooms
                .iter()
                .find(|row| row.room_id == room_id && row.membership == RoomMembership::Joined)
                .map(|_| ())
        })
        .await;
}

/// 打开着的私聊，它的列表行在同步重启里被错标成受邀再恢复几次，订阅不能跟着退：只要房间还打开着，
/// 房间列表连接的每一次增量同步都带着它的订阅。卡片不再走 Matrix（ADR-0016），客户端不再关注核心的
/// 私聊，没打开的房间没有这份订阅，也就没有靠关注集合撑着的那一段了。
#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn the_room_stays_subscribed_while_its_row_flickers() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let home = Home::start(&[phone.device_id()]).await;
    phone.connect(&home);
    phone.signed_in().await;
    let core = home.bot("wonton-core", "Wonton").await;
    let hermes = home.bot("hermes", "Hermes").await;
    let room_id = joined_direct_chat(&phone, &core).await;
    // 戳长轮询用的另一个房间：往它发消息结束在飞的同步，不动私聊的时间线，加入事件才还是私聊里最新的事件。
    let poke = joined_direct_chat(&phone, &hermes).await;
    let poke_room = hermes.open_room(&poke).await.expect("Bot 打开戳人的房间");

    // 打开房间：卡片改从列表来之后，这是订阅唯一的来源。
    let _timeline = Timeline::open(&phone, room_id.as_str()).await;
    // 打开着的抖动：行错标成受邀再恢复，订阅不能跟着退。
    flicker(&phone, &home, &poke_room, room_id.as_str(), "打开着").await;

    // 恢复名单和启动串行，打开之后每一次同步都得带着订阅。加入流程可能留下一段临时订阅
    // （`until_joined` 的重订阅窗口，收尾那条又不带），定位从最后一条不带订阅的请求之后开始，不被它咬到；
    // 最后一条也要带着：订阅要是丢了就回不来，不能被空循环放过。
    let requests = room_list_requests(&home, 0);
    let start = requests
        .iter()
        .rposition(|(_, subs)| !subs.iter().any(|room| room.as_str() == room_id.as_str()))
        .map_or(0, |last_absent| last_absent + 1);
    let (_, tail) = requests.last().expect("有过房间列表的同步请求");
    assert!(
        tail.iter().any(|room| room.as_str() == room_id.as_str()),
        "最后的同步请求丢了 {room_id} 的订阅：{tail:?}; all requests: {:?}",
        home.sync_requests()
    );
    for (i, (pos, subs)) in requests.iter().enumerate().skip(start) {
        assert!(
            subs.iter().any(|room| room.as_str() == room_id.as_str()),
            "sync request {i} (pos={pos:?}) lost the subscription for {room_id}: {subs:?}; all requests: {:?}",
            home.sync_requests()
        );
    }
}

/// 同步重启一次（`M_UNKNOWN_POS`）之后，打开着的房间的订阅要重新挂上：重启后的请求带着它，
/// Bot 发的新消息能进入打开着的时间线。
#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn a_sync_restart_reapplies_the_open_rooms_subscription() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let home = Home::start(&[phone.device_id()]).await;
    phone.connect(&home);
    phone.signed_in().await;
    let bot = home.bot("hermes", "Hermes").await;
    let room_id = joined_direct_chat(&phone, &bot).await;
    let bot_room = bot.open_room(&room_id).await.expect("Bot 打开私聊");
    bot_room
        .send_text("重启前的第一条")
        .await
        .expect("发第一条");
    bot_room
        .send_text("重启前的第二条")
        .await
        .expect("发第二条");
    let mut timeline = Timeline::open(&phone, room_id.as_str()).await;
    timeline
        .until("打开时看到历史", |item| text(item, "重启前的第二条"))
        .await;

    // 注入的是 arm 之后第一条房间列表请求：在飞的长轮询被新消息结束，紧接着的这条拿到 M_UNKNOWN_POS。
    let injected = home
        .sync_requests()
        .last()
        .map(|request| request["test_sequence"].as_u64().unwrap())
        .unwrap_or(0);
    home.expire_room_list_sync();
    bot_room
        .send_text("戳一下长轮询")
        .await
        .expect("戳一下同步");
    // 叫醒等退避的重启，不等间隔。
    phone.dispatch(Action::AppForegrounded).unwrap();
    wait_for_sync("重启后订阅补挂", &home, || {
        let requests = room_list_requests(&home, injected);
        // 重启的首条请求无 pos，首条和后续带 pos 的请求都必须保留订阅。
        let initial = requests.iter().position(|(pos, _)| pos.is_none())?;
        assert!(
            requests[initial]
                .1
                .iter()
                .any(|room| room.as_str() == room_id.as_str()),
            "the first request after restart must retain the open room subscription: {requests:?}"
        );
        requests
            .iter()
            .skip(initial + 1)
            .find(|(pos, subs)| {
                pos.is_some() && subs.iter().any(|room| room.as_str() == room_id.as_str())
            })
            .map(|_| ())
    })
    .await;

    bot_room
        .send_text("重启后 Bot 发的")
        .await
        .expect("Bot 重启后发消息");
    timeline
        .until("重启后收到 Bot 的新消息", |item| {
            text(item, "重启后 Bot 发的")
        })
        .await;
    // 更早的历史在重启重建后的时间线里要不要立刻显示，是上游时间线重建的语义（#370 查过，归上游），
    // 这里不断言它；订阅补挂本身由上面的请求断言，新消息能进来由这一条断言。
}
