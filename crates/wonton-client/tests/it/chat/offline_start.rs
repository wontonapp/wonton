//! 打开 App 先用本机存下的聊天：隧道还没通（家里离线、绑定卡着）时也立刻给出房间和历史，通上以后再同步。
//!
//! 本机有会话、没有本地代理时照样恢复：`proxy: None` 的会话，请求按传输错误失败，状态如实显示，同步自己重试。

use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use wonton_client::{ChatAccount, ChatSync};

use super::support::{Home, Phone, Timeline};
use super::{accept_the_bots_invite, chat_with_the_bot, text};

/// 绑定的闸门：关着时 iroh 的绑定不返回，隧道和本地代理都出不来（家里离线、绑定又慢的样子）。
/// 绑定钩子每次绑定前读一次，开了以后下一轮重试就绑得上。
#[derive(Clone)]
struct Gate(Arc<(Mutex<bool>, Condvar)>);

impl Gate {
    fn closed() -> Self {
        Self(Arc::new((Mutex::new(true), Condvar::new())))
    }

    fn hook(&self) -> Arc<dyn Fn() + Send + Sync> {
        let gate = self.clone();
        Arc::new(move || {
            let (closed, wake) = &*gate.0;
            let mut closed = closed.lock().unwrap();
            while *closed {
                closed = wake.wait(closed).unwrap();
            }
        })
    }

    fn open(&self) {
        let (closed, wake) = &*self.0;
        *closed.lock().unwrap() = false;
        wake.notify_all();
    }
}

/// 重启后连不上家里（绑定卡着）：本机存着的房间和历史先出来，不用等隧道；通上以后同步接着跑、新消息到达。
#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn a_restart_without_the_tunnel_shows_the_local_rooms_and_history() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let home = Home::start(&[phone.device_id()]).await;
    phone.connect(&home);
    phone.signed_in().await;
    let bot = home.bot("hermes", "Hermes").await;
    let room_id = accept_the_bots_invite(&home, &phone, &bot).await;
    chat_with_the_bot(&phone, &bot, &room_id).await;
    let login_tokens = home.login_tokens();
    phone.test_shut_down().await;
    drop(phone);

    // 重启时连接信息还没到（App 先开界面、后连家里），绑定也卡着：本地代理出不来。
    std::fs::remove_file(dir.path().join("servers/1/home-ticket.json")).expect("删掉连接信息");
    let gate = Gate::closed();
    let phone = Phone::open(dir.path());
    phone.test_set_bind_hook(Some(gate.hook()));
    phone.test_set_establish_timing(Duration::from_millis(200), Duration::from_millis(100));
    phone.connect(&home);

    // 不等隧道：恢复本机会话，房间行和已有的历史都在，账号是登录着的。
    phone
        .wait_chat_within(
            "本机的房间和历史先出来",
            Duration::from_secs(5),
            |view| {
                (matches!(view.account, ChatAccount::SignedIn { .. })
                    && view.rooms.iter().any(|row| row.room_id == room_id.as_str()))
                .then_some(())
            },
        )
        .await;
    assert!(phone.home().proxy.is_none(), "隧道还没通，不该有本地代理");
    let mut timeline = Timeline::open(&phone, room_id.as_str()).await;
    timeline
        .until("本机的历史还在", |item| text(item, "你好，主人"))
        .await;

    // 隧道通了：同步跑起来，新消息到达，恢复用的还是存下的会话。
    gate.open();
    phone
        .wait_chat("隧道通了，同步跑起来", |view| {
            (view.sync == ChatSync::Running).then_some(())
        })
        .await;
    let mut timeline = Timeline::open(&phone, room_id.as_str()).await;
    let bot_room = bot.open_room(&room_id).await.expect("Bot 打开私聊");
    bot_room.send_text("连上了", &[]).await.expect("Bot 发消息");
    timeline
        .until("恢复后收到新消息", |item| text(item, "连上了"))
        .await;
    assert_eq!(
        home.login_tokens(),
        login_tokens,
        "恢复会话不应再要登录令牌"
    );
}
/// 本机没有会话（新设备）：隧道没通时照旧停在“还没登录”，不显示本地代理的错误，也不来回重连。
#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn a_device_without_a_stored_session_waits_for_the_tunnel() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let home = Home::start(&[phone.device_id()]).await;
    let gate = Gate::closed();
    phone.test_set_bind_hook(Some(gate.hook()));
    phone.test_set_establish_timing(Duration::from_millis(200), Duration::from_millis(100));
    phone.connect(&home);

    tokio::time::sleep(Duration::from_millis(1500)).await;
    let view = phone.chat();
    assert!(
        matches!(view.account, ChatAccount::SignedOut),
        "没有会话时不该自己进登录/重连：{:?}",
        view.account
    );
    assert_eq!(view.sync, ChatSync::Idle);

    gate.open();
    let (user_id, _) = phone.signed_in().await;
    assert_eq!(user_id, super::support::OWNER);
}
