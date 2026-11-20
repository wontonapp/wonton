use wonton_client::{Action, HomeStatus, PairingStage, PairingStatus};

use super::support::{Home, Phone, SLOW_REPLY, paired};

/// 慢回应：配对码交出后进度翻到“等服务器确认”；取消回到可以重新输入的状态，服务端迟到的登记不算数，
/// 同一个码再配一次，服务端对已登记的设备幂等地回成功。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancelling_a_slow_pairing_returns_to_the_form_and_a_retry_pairs() {
    let home_side = Home::start().await;
    *home_side.codes.slow.lock().unwrap() = 1;
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let code = home_side.code(None);
    phone.pair(&code).unwrap();
    // 直连建连很快，码交出就翻步；时间由心跳按秒推进，第一次翻步时至少过了一秒。
    phone
        .wait_for(|view| {
            matches!(
                view.pairing,
                Some(PairingStatus::InProgress {
                    elapsed_ms: 1_000..,
                    stage: PairingStage::Confirming
                })
            )
        })
        .await;
    phone
        .dispatch(Action::Home {
            action: wonton_protocol::HomeAction::CancelPairing,
        })
        .unwrap();
    assert!(phone.home().pairing.is_none());
    assert_eq!(phone.home().status, HomeStatus::NotConfigured);
    // 取消不是配上：核心不给 Paired，壳的“配上了”只看这个类别，取消回到没有进展。
    assert!(phone.home().server_id.is_none());
    // 迟到的回应不算数：取消之后状态不再变。
    tokio::time::sleep(SLOW_REPLY).await;
    assert!(phone.home().pairing.is_none(), "{:?}", phone.home());
    phone.pair(&code).unwrap();
    let view = phone.wait_for(paired).await;
    assert_eq!(
        view.server_id,
        Some(home_side.server.endpoint().id().to_string())
    );
}
