use super::*;

async fn paused_home() -> (
    tempfile::TempDir,
    Arc<HomeLink>,
    watch::Receiver<PushMethod>,
) {
    let dir = tempfile::tempdir().unwrap();
    let storage = Storage::open(dir.path(), dir.path()).unwrap();
    let (_method_tx, method_rx) = watch::channel(PushMethod::default());
    let home = HomeLink::new(
        tokio::runtime::Handle::current(),
        wonton_tunnel::sharing::ProxyHub::bind().await.unwrap(),
        storage,
        Network::DirectOnly,
        None,
        method_rx.clone(),
    )
    .unwrap();
    (dir, home, method_rx)
}

#[tokio::test(start_paused = true)]
async fn a_media_pause_keeps_checking_for_a_minute() {
    let (_dir, home, _method_rx) = paused_home().await;
    home.backgrounded();
    home.page_media_active(true);
    home.page_media_active(false);
    tokio::time::sleep(Duration::from_secs(59)).await;
    assert!(!*home.background.borrow(), "59 秒内还应保活");
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert!(*home.background.borrow(), "过了宽限期回到后台");
}

#[tokio::test(start_paused = true)]
async fn going_to_the_background_during_the_grace_keeps_checking() {
    let (_dir, home, _method_rx) = paused_home().await;
    home.backgrounded();
    home.page_media_active(true);
    home.page_media_active(false);
    tokio::time::sleep(Duration::from_secs(10)).await;
    home.backgrounded();
    tokio::time::sleep(Duration::from_secs(49)).await;
    assert!(!*home.background.borrow(), "宽限期内切后台仍然保活");
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert!(*home.background.borrow(), "宽限期结束才回到后台");
}

#[tokio::test(start_paused = true)]
async fn a_woken_release_does_not_override_playback() {
    let (_dir, home, _method_rx) = paused_home().await;
    home.backgrounded();
    home.page_media_active(true);
    home.page_media_active(false);
    tokio::time::sleep(Duration::from_secs(61)).await;
    assert!(*home.background.borrow(), "宽限期先结束");
    home.page_media_active(true);
    tokio::time::sleep(Duration::from_secs(90)).await;
    assert!(!*home.background.borrow(), "旧释放任务不再把保活关掉");
}

#[tokio::test(start_paused = true)]
async fn playing_again_cancels_the_release() {
    let (_dir, home, _method_rx) = paused_home().await;
    home.backgrounded();
    home.page_media_active(true);
    home.page_media_active(false);
    tokio::time::advance(Duration::from_secs(30)).await;
    home.page_media_active(true);
    tokio::time::advance(Duration::from_secs(90)).await;
    tokio::task::yield_now().await;
    assert!(!*home.background.borrow());
}

#[tokio::test(start_paused = true)]
async fn page_media_keeps_the_tunnel_checked_in_the_background() {
    let (_dir, home, _method_rx) = paused_home().await;
    home.backgrounded();
    assert!(*home.background.borrow());
    home.page_media_active(true);
    assert!(!*home.background.borrow());
    home.page_media_active(false);
    assert!(!*home.background.borrow(), "宽限期内仍然保活");
    tokio::time::sleep(Duration::from_secs(61)).await;
    assert!(*home.background.borrow());
    home.foregrounded();
    assert!(!*home.background.borrow());
    home.backgrounded();
    assert!(*home.background.borrow());
    home.page_media_active(true);
    home.foregrounded();
    home.backgrounded();
    assert!(!*home.background.borrow());
}

/// 核心自己经隧道请求的闸门：已知断开给原因，连接中、已连上和还没配对不拦。
#[tokio::test(start_paused = true)]
async fn a_known_disconnected_connection_gates_core_requests() {
    let (_dir, home, _method_rx) = paused_home().await;
    assert_eq!(home.tunnel_outage(), None, "还没配对不拦");
    for status in [HomeStatus::Connecting, HomeStatus::Connected] {
        home.view.send_modify(|view| view.status = status.clone());
        assert_eq!(home.tunnel_outage(), None, "{status:?} 不拦");
    }
    for reason in [HomeFailure::TransportUnavailable, HomeFailure::Unauthorized] {
        home.view.send_modify(|view| {
            view.status = HomeStatus::Disconnected {
                reason,
                message: "测试".into(),
            }
        });
        assert_eq!(home.tunnel_outage(), Some(reason), "{reason:?} 要拦");
    }
}
