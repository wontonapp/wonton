use wonton_client::{
    Action, ChatAccount, HomeStatus, PairingFailure, PairingStage, PairingStatus, TabRef,
};
use wonton_tunnel::pair::RejectReason;

use super::support::{Home, Phone, paired};

/// 手机上已经配好一台服务器时再扫一个会被服务端拒绝的码：`PairHome` 同步返回成功，槽位当场就换了，
/// 拒绝要等服务端回话才出现；拒绝之后停在新槽、不回到原来那台，原来开着的那一页也没了。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pairing_again_switches_the_slot_before_the_server_rejects() {
    let home = Home::start().await;
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    phone.pair(&home.code(None)).unwrap();
    phone.wait_for(paired).await;
    let old = phone
        .servers()
        .servers
        .iter()
        .find(|server| server.current)
        .unwrap()
        .id
        .clone();
    phone
        .dispatch(Action::Pages {
            action: wonton_protocol::PageAction::OpenPage {
                tab: TabRef::Url {
                    url: "https://example.test/".to_owned(),
                },
            },
        })
        .unwrap();
    assert_eq!(phone.pages().stack.len(), 1);

    // 服务端拖住回应：核心这边已经同步成功，结论还没到。
    *home.codes.slow.lock().unwrap() = 1;
    phone.pair(&home.code(Some(RejectReason::Expired))).unwrap();
    let view = phone
        .wait_for(|view| {
            matches!(
                view.pairing,
                Some(PairingStatus::InProgress {
                    stage: PairingStage::Confirming,
                    ..
                })
            )
        })
        .await;
    // 槽位在服务端回话前就换了：当前已经是不配对的新槽位，旧的那台不再挂着，界面停在配对页。
    assert_eq!(view.status, HomeStatus::NotConfigured);
    assert!(view.server_id.is_none());
    let servers = phone.servers();
    assert_eq!(servers.servers.len(), 1);
    assert_eq!(servers.servers[0].id, old);
    assert!(!servers.servers[0].current);
    assert!(servers.adding);
    assert!(matches!(phone.chat().account, ChatAccount::SignedOut));
    assert!(
        phone.pages().stack.is_empty(),
        "原来开着的那一页已经随旧槽位没了"
    );

    let (reason, _) = phone.pairing_failure().await;
    assert_eq!(reason, PairingFailure::Expired);
    assert!(phone.home().pairing.is_some());
    assert_eq!(phone.home().status, HomeStatus::NotConfigured);
    // 拒绝之后也没有自动回到原来那台：旧服务器仍不挂着，手机得自己取消或重试。
    let servers = phone.servers();
    assert_eq!(servers.servers.len(), 1);
    assert_eq!(servers.servers[0].id, old);
    assert!(!servers.servers[0].current);
    assert!(servers.adding);
    assert!(phone.home().server_id.is_none());
    // 新槽没有登录，谈不上交接页。
    assert!(phone.chat().encryption.key_handoff.is_none());
}
