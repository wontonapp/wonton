//! 一台手机配多台服务器（ADR-0042）：同一时间连一台，其他几台的数据留在手机上。只经由导出给原生壳的接口驱动，
//! 隧道服务端跑在本机。聊天的部分（切回来不用重新登录）在 `chat::server_switch`，传输的部分在 `files`。

use wonton_client::{Action, CoreError, HomeStatus, ServerRow, SettingsEntry};

use crate::home::{Home, Phone};

fn row(id: &str, name: &str, current: bool) -> ServerRow {
    ServerRow {
        id: id.to_owned(),
        name: name.to_owned(),
        current,
    }
}

fn server_id(home: &Home) -> String {
    home.server().endpoint().id().to_string()
}

async fn connected_to(phone: &Phone, home: &Home) {
    let id = server_id(home);
    phone
        .wait_for(|view| {
            view.status == HomeStatus::Connected && view.server_id.as_deref() == Some(id.as_str())
        })
        .await;
}

async fn wait_unpaired(phone: &Phone) {
    phone
        .wait_for(|view| view.status == HomeStatus::NotConfigured && view.server_id.is_none())
        .await;
}

/// 配好两台、切回第一台：两台都在列表里，各用各的名字；重启后还在第一台上。
#[tokio::test]
async fn a_second_server_is_added_beside_the_first_and_both_are_kept() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let a = Home::start(&[phone.device_id()]).await;
    let b = Home::start(&[phone.device_id()]).await;
    assert!(
        matches!(
            phone.dispatch(Action::AddServer),
            Err(CoreError::InvalidInput { .. })
        ),
        "还没配好第一台"
    );
    phone.connect(&a);
    connected_to(&phone, &a).await;
    assert_eq!(phone.servers().servers, [row("1", "家里的服务器", true)]);

    phone.dispatch(Action::AddServer).unwrap();
    wait_unpaired(&phone).await;
    assert!(phone.servers().adding);
    phone.connect(&b);
    connected_to(&phone, &b).await;
    wait_servers(&phone, |servers| !servers.adding).await;
    assert_eq!(
        phone.servers().servers,
        [row("1", "家里的服务器", false), row("2", "服务器 2", true)]
    );

    phone
        .dispatch(Action::SwitchServer { id: "1".into() })
        .unwrap();
    connected_to(&phone, &a).await;
    drop(phone);
    let phone = Phone::open(dir.path());
    connected_to(&phone, &a).await;
    assert_eq!(
        phone.servers().servers,
        [row("1", "家里的服务器", true), row("2", "服务器 2", false)]
    );
}

/// 名字写在设置首页的服务器一行上（不止一台时），改名马上生效；空名字不收。
#[tokio::test]
async fn the_current_servers_name_follows_renames() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let a = Home::start(&[phone.device_id()]).await;
    let b = Home::start(&[phone.device_id()]).await;
    phone.connect(&a);
    connected_to(&phone, &a).await;
    assert_eq!(server_name(&phone), None, "只有一台时不写名字");
    phone.dispatch(Action::AddServer).unwrap();
    wait_unpaired(&phone).await;
    phone.connect(&b);
    wait_servers(&phone, |servers| servers.servers.len() == 2).await;

    phone
        .dispatch(Action::RenameServer {
            id: "2".into(),
            name: " 办公室 ".into(),
        })
        .unwrap();
    assert_eq!(phone.servers().servers[1], row("2", "办公室", true));
    wait_settings(&phone, |name| name == Some("办公室")).await;
    for (id, name) in [("2", "  "), ("9", "别处")] {
        let renamed = phone.dispatch(Action::RenameServer {
            id: id.into(),
            name: name.into(),
        });
        assert!(
            matches!(renamed, Err(CoreError::InvalidInput { .. })),
            "{id} {name}"
        );
    }
}

/// 添加到一半取消：回到原来那台，配对中的位置不留。
#[tokio::test]
async fn cancelling_an_add_returns_to_the_server_you_were_on() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let a = Home::start(&[phone.device_id()]).await;
    phone.connect(&a);
    connected_to(&phone, &a).await;

    phone.dispatch(Action::AddServer).unwrap();
    wait_unpaired(&phone).await;
    assert!(dir.path().join("servers/2").exists());
    phone.dispatch(Action::CancelAddServer).unwrap();

    connected_to(&phone, &a).await;
    let servers = phone.servers();
    assert!(!servers.adding);
    assert_eq!(servers.servers, [row("1", "家里的服务器", true)]);
    assert!(!dir.path().join("servers/2").exists());
}

/// 移除只删手机上的数据。移除当前这台切到剩下的；一台不剩回到配对页。
#[tokio::test]
async fn removing_servers_deletes_their_data_and_the_last_one_returns_to_pairing() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let a = Home::start(&[phone.device_id()]).await;
    let b = Home::start(&[phone.device_id()]).await;
    phone.connect(&a);
    connected_to(&phone, &a).await;
    phone.dispatch(Action::AddServer).unwrap();
    wait_unpaired(&phone).await;
    phone.connect(&b);
    wait_servers(&phone, |servers| servers.servers.len() == 2).await;

    phone
        .dispatch(Action::RemoveServer { id: "2".into() })
        .unwrap();
    connected_to(&phone, &a).await;
    assert_eq!(phone.servers().servers, [row("1", "家里的服务器", true)]);
    assert!(!dir.path().join("servers/2").exists());

    phone
        .dispatch(Action::RemoveServer { id: "1".into() })
        .unwrap();
    wait_unpaired(&phone).await;
    assert!(phone.servers().servers.is_empty());
    assert!(!phone.servers().adding, "没有可以回去的，配对页上不给取消");
    assert!(!dir.path().join("servers/1").exists());
}

/// 同一台服务器再配一次：用新位置，旧位置删掉，名字留着。
#[tokio::test]
async fn pairing_the_same_server_again_replaces_its_old_place() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let a = Home::start(&[phone.device_id()]).await;
    phone.connect(&a);
    connected_to(&phone, &a).await;

    phone.dispatch(Action::AddServer).unwrap();
    wait_unpaired(&phone).await;
    phone.connect(&a);

    wait_servers(&phone, |servers| {
        servers.servers == [row("2", "家里的服务器", true)] && !servers.adding
    })
    .await;
    let old = dir.path().join("servers/1");
    tokio::time::timeout(std::time::Duration::from_secs(30), async {
        while old.exists() {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("旧位置应删掉");
}

fn server_name(phone: &Phone) -> Option<String> {
    phone
        .settings()
        .sections
        .into_iter()
        .flat_map(|section| section.entries)
        .find_map(|entry| match entry {
            SettingsEntry::Server { name, .. } => Some(name),
            _ => None,
        })
        .flatten()
}

async fn wait_servers(
    phone: &Phone,
    mut done: impl FnMut(&wonton_client::ServersViewModel) -> bool,
) {
    let subscription = phone.subscribe_servers();
    let waiting = async {
        while let Some(view) = subscription.next().await {
            if done(&view) {
                return;
            }
        }
        panic!("订阅提前结束");
    };
    tokio::time::timeout(std::time::Duration::from_secs(30), waiting)
        .await
        .unwrap_or_else(|_| panic!("等待超时，当前：{:?}", phone.servers()));
}

async fn wait_settings(phone: &Phone, mut done: impl FnMut(Option<&str>) -> bool) {
    let subscription = phone.subscribe_settings();
    let waiting = async {
        while subscription.next().await.is_some() {
            if done(server_name(phone).as_deref()) {
                return;
            }
        }
        panic!("订阅提前结束");
    };
    tokio::time::timeout(std::time::Duration::from_secs(30), waiting)
        .await
        .unwrap_or_else(|_| panic!("等待超时，当前：{:?}", server_name(phone)));
}
