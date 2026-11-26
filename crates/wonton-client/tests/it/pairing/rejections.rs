use wonton_client::{Action, HomeStatus, PairingFailure};
use wonton_tunnel::pair::RejectReason;

use super::support::{Home, Phone, paired};

#[tokio::test]
async fn a_rejected_code_says_what_to_do_next() {
    let home = Home::start().await;
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    for (outcome, failure) in [
        (Some(RejectReason::UnknownCode), PairingFailure::Rejected),
        (Some(RejectReason::Expired), PairingFailure::Expired),
        (Some(RejectReason::Used), PairingFailure::Used),
        (Some(RejectReason::Closed), PairingFailure::Closed),
        (Some(RejectReason::BadRequest), PairingFailure::Protocol),
        (Some(RejectReason::ServerError), PairingFailure::Upstream),
        (Some(RejectReason::Other), PairingFailure::Rejected),
    ] {
        let code = home.code(outcome);
        phone.pair(&code).unwrap();
        let (got, message) = phone.pairing_failure().await;
        assert_eq!(got, failure, "{outcome:?}: {message}");
        assert_eq!(
            phone.home().status,
            HomeStatus::NotConfigured,
            "配不上就不存连接信息"
        );
        phone
            .dispatch(Action::Home {
                action: wonton_protocol::HomeAction::DismissPairingFailure,
            })
            .unwrap();
        assert!(phone.home().pairing.is_none());
        phone.pair(&code).unwrap();
        let (rescanned, message) = phone.pairing_failure().await;
        assert_eq!(rescanned, failure, "重新扫描同一个码：{message}");
    }
    // 拒绝之后换一个好码照样能配上。
    phone.pair(&home.code(None)).unwrap();
    phone.wait_for(paired).await;
}
