//! 配好、跳过交接后进程被杀，再扫一次自己的家：登录成功重新点起交接，正进行着再点“跳过”就地结束，不回恢复密钥。

use wonton_client::{
    Action, BackupReminder, KeyHandoff, PairingStage, PairingStatus, SecretString, TabRef,
};

use super::super::support::{Home, Phone, WAIT};
use super::{old_phone, pair};

/// 走查的第 5 步：已经配好、交接也已经跳过，进程被杀后再扫一次自己的家。`PairHome` 同步成功就把槽位换掉，
/// 服务端回话和登录都还在后台；登录成功会重新点起交接页，主人再点“跳过”时那一次点在活动中的交接上，
/// 只能原地结束，不能再把壳切回恢复密钥。
#[tokio::test(flavor = "multi_thread")]
#[ignore = "需要 Docker，用 --include-ignored 运行"]
async fn pairing_again_arms_the_handoff_and_skipping_afterwards_ends_it() {
    let old_dir = tempfile::tempdir().unwrap();
    let new_dir = tempfile::tempdir().unwrap();
    let old = Phone::open(old_dir.path());
    let new = Phone::open(new_dir.path());
    let home = Home::start(&[old.device_id()]).await;
    old_phone(&home, &old, true).await;
    pair(&new, home.pairing_code());
    new.signed_in().await;
    new.wait_chat("第一次交接出现", |view| {
        matches!(
            view.encryption.key_handoff,
            Some(KeyHandoff::Receiving { .. })
        )
        .then_some(())
    })
    .await;
    new.dispatch(Action::Chat {
        action: wonton_protocol::ChatAction::SkipKeyHandoff,
    })
    .unwrap();
    new.wait_chat("第一次交接跳过", |view| {
        (view.encryption.key_handoff.is_none()
            && view.encryption.backup_reminder == Some(BackupReminder::EnterRecoveryKey))
        .then_some(())
    })
    .await;

    let tokens = pair_again_while_server_stalls(&new, &home).await;

    // 服务端回话、登录成功：交接页重新点起来。
    new.wait_chat("配上并重新登录后再次出现交接", |view| {
        matches!(
            view.encryption.key_handoff,
            Some(KeyHandoff::Receiving { .. })
        )
        .then_some(())
    })
    .await;
    assert!(home.login_tokens() > tokens);
    // 配上后同一台的旧槽位被删掉，只剩登录成功的这一个。
    let servers = new.servers();
    assert_eq!(servers.servers.len(), 1);
    assert!(servers.servers[0].current);
    let marker = new_dir
        .path()
        .join("servers")
        .join(&servers.servers[0].id)
        .join("matrix")
        .join("key-handoff");
    assert!(marker.is_file(), "新槽的交接标记在");

    // 交接正进行着就跳过：接管这一次，标记清掉，不再回到恢复密钥。
    new.dispatch(Action::Chat {
        action: wonton_protocol::ChatAction::SkipKeyHandoff,
    })
    .unwrap();
    assert!(!marker.exists(), "跳过就地清掉标记");
    new.wait_chat("跳过后停住", |view| {
        (view.encryption.key_handoff.is_none()
            && view.encryption.verification_flow.is_none()
            && view.encryption.backup_reminder == Some(BackupReminder::EnterRecoveryKey))
        .then_some(())
    })
    .await;
    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
    assert!(
        new.chat().encryption.key_handoff.is_none(),
        "不会再点起一次：{:?}",
        new.chat().encryption.key_handoff
    );
    assert!(!marker.exists());
    assert_eq!(home.login_tokens(), tokens + 1, "也没再登录一次");
}

/// 让服务端拖住下一次配对的回应：同步换槽要发生在回话和登录之前。返回配对前的登录次数。
async fn pair_again_while_server_stalls(new: &Phone, home: &Home) -> usize {
    let old_slot = new
        .servers()
        .servers
        .iter()
        .find(|server| server.current)
        .unwrap()
        .id
        .clone();
    let tokens = home.login_tokens();
    new.dispatch(Action::Pages {
        action: wonton_protocol::PageAction::OpenPage {
            tab: TabRef::Url {
                url: "https://example.test/".to_owned(),
            },
        },
    })
    .unwrap();
    assert_eq!(new.pages().stack.len(), 1);

    // 服务端拖住回应：同步成功已经换了槽，登录和交接都还没开始。
    home.slow_next_pairing();
    new.dispatch(Action::Home {
        action: wonton_protocol::HomeAction::PairHome {
            code: SecretString::new(home.pairing_code()),
            device_name: "new-phone".to_owned(),
        },
    })
    .unwrap();
    let subscription = new.subscribe_home();
    let waiting = async {
        loop {
            let view = subscription.next().await.expect("配对状态订阅提前结束");
            if matches!(
                view.pairing,
                Some(PairingStatus::InProgress {
                    stage: PairingStage::Confirming,
                    ..
                })
            ) {
                return;
            }
        }
    };
    tokio::time::timeout(WAIT, waiting)
        .await
        .unwrap_or_else(|_| panic!("没等到服务端确认：{:?}", new.home().pairing));
    assert!(new.chat().encryption.key_handoff.is_none());
    // 槽位在服务端回话前就换了：当前已经是不配对的新槽位，旧的那台不再挂着。
    let servers = new.servers();
    assert_eq!(servers.servers.len(), 1);
    assert_eq!(servers.servers[0].id, old_slot);
    assert!(!servers.servers[0].current);
    assert!(servers.adding);
    assert!(new.home().server_id.is_none());
    assert_eq!(home.login_tokens(), tokens, "还没登录");
    assert!(new.pages().stack.is_empty(), "原来开着的那一页随旧槽位没了");

    tokens
}
