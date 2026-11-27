//! “我的应用”往账号数据的同步（ADR-0052）：隧道一直连着、聊天沿用原来的会话时，没写上的改动也要自己补上，
//! 补上了侧边栏的“改动还没同步上”就收掉。

use std::time::Duration;

use wonton_client::{Action, HomeStatus, MyAppRef, PageAction, PagesViewModel, TabRef};

use super::support::{Home, Phone, WAIT};

fn add_page(phone: &Phone, url: &str) {
    phone
        .dispatch(Action::Pages {
            action: PageAction::AddMyApp {
                app: MyAppRef::Page {
                    tab: TabRef::Url {
                        url: url.to_owned(),
                    },
                },
                title: Some(url.to_owned()),
                index: None,
            },
        })
        .unwrap();
}

async fn wait_pages(phone: &Phone, what: &str, mut done: impl FnMut(&PagesViewModel) -> bool) {
    let deadline = tokio::time::Instant::now() + WAIT;
    loop {
        let view = phone.pages();
        if done(&view) {
            return;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "{what}：{WAIT:?} 内没有等到，当前：{view:?}"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

async fn wait_pushed(phone: &Phone, what: &str) {
    let deadline = tokio::time::Instant::now() + WAIT;
    while phone.test_pages_layout_dirty() {
        assert!(tokio::time::Instant::now() < deadline, "{what}");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn an_unsynced_change_is_pushed_once_the_account_data_answers_again() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let home = Home::start(&[phone.device_id()]).await;
    phone.connect(&home);
    let signed_in = phone.signed_in().await;

    add_page(&phone, "https://example.com/first");
    wait_pushed(&phone, "连着时的改动直接写上").await;
    assert!(!phone.pages().sync_stale);

    // 隧道和会话都在，只有账号数据的请求断在半路。
    home.set_my_apps_down(true);
    add_page(&phone, "https://example.com/second");
    wait_pages(&phone, "写没成要说还没同步上", |view| {
        view.sync_stale
    })
    .await;
    assert_eq!(phone.home().status, HomeStatus::Connected);
    let titles: Vec<String> = phone
        .plugins()
        .sidebar
        .apps
        .iter()
        .map(|app| app.title.clone())
        .collect();
    assert!(
        titles.iter().any(|title| title.contains("second")),
        "本机存好了：{titles:?}"
    );

    home.set_my_apps_down(false);
    wait_pages(
        &phone,
        "账号数据好了自己补上，提示收掉",
        |view| !view.sync_stale,
    )
    .await;
    wait_pushed(&phone, "没写上的改动推上去了").await;
    assert_eq!(phone.signed_in().await, signed_in, "会话没换也能补上");
}
