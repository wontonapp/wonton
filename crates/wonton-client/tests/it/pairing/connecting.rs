use std::net::Ipv4Addr;
use std::time::Duration;

use iroh::SecretKey;
use url::Url;
use wonton_client::{Action, CoreError, HomeStatus, PairingFailure, PairingStage, PairingStatus};
use wonton_tunnel::pair::{PairingCode, PairingSecret, SECRET_LEN};

use super::support::Phone;

#[tokio::test]
async fn unreachable_pairing_stops_before_cleanup_and_retry_starts_again() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let dead = std::net::UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let code = PairingCode {
        server_id: SecretKey::generate().public(),
        relays: Vec::new(),
        direct_addrs: vec![dead.local_addr().unwrap()],
        base_domain: "wonton.localhost".into(),
        core_url: Url::parse("http://wonton.localhost:18480/").unwrap(),
        secret: PairingSecret::from_bytes([42; SECRET_LEN]),
    }
    .to_uri();
    phone.pair(&code).unwrap();
    assert!(matches!(
        phone.home().pairing,
        Some(PairingStatus::InProgress {
            stage: PairingStage::Connecting,
            ..
        })
    ));
    // 建连那一段 30 秒，超时就报连不上（不再确认）。几秒只给调度和 QUIC 清理留余量：
    // 清理不能推迟可见的结果。
    let (failure, message) = tokio::time::timeout(Duration::from_secs(32), phone.pairing_failure())
        .await
        .expect("pairing failure must be published before endpoint cleanup");
    assert_eq!(failure, PairingFailure::TransportUnavailable, "{message}");
    let Some(PairingStatus::Failed {
        message,
        diagnostic,
        ..
    }) = phone.home().pairing
    else {
        unreachable!()
    };
    let diagnostic = diagnostic.expect("原始诊断仍可获得");
    assert!(
        !message.contains("tunnel transport unavailable"),
        "{message}"
    );
    assert!(
        !message.contains(&dead.local_addr().unwrap().to_string()),
        "{message}"
    );
    assert!(diagnostic.contains("transport-unavailable"), "{diagnostic}");
    assert!(
        diagnostic.contains(&dead.local_addr().unwrap().to_string()),
        "{diagnostic}"
    );
    assert!(!diagnostic.contains(PairingCode::parse(&code).unwrap().secret.expose()));
    assert_eq!(phone.home().status, HomeStatus::NotConfigured);
    assert!(phone.home().server_id.is_none());
    phone
        .dispatch(Action::Home {
            action: wonton_protocol::HomeAction::DismissPairingFailure,
        })
        .unwrap();
    assert!(phone.home().pairing.is_none());
    assert_eq!(phone.home().status, HomeStatus::NotConfigured);
    phone.pair(&code).unwrap();
    assert!(matches!(
        phone.home().pairing,
        Some(PairingStatus::InProgress { .. })
    ));
}

/// 连不上的地址：配对码交不出去，进度一直停在“正在连接”，已等待的时间由核心按秒推进——壳的“还在连接”
/// 和取消按钮就从这个状态来。取消后回到没有配对的状态；没有配对时取消是输入错误。
#[tokio::test]
async fn an_unreachable_server_stays_connecting_and_the_wait_grows() {
    let dir = tempfile::tempdir().unwrap();
    let phone = Phone::open(dir.path());
    let dead = std::net::UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let code = PairingCode {
        server_id: SecretKey::generate().public(),
        relays: Vec::new(),
        direct_addrs: vec![dead.local_addr().unwrap()],
        base_domain: "wonton.localhost".into(),
        core_url: Url::parse("http://wonton.localhost:18480/").unwrap(),
        secret: PairingSecret::from_bytes([42; SECRET_LEN]),
    }
    .to_uri();
    phone.pair(&code).unwrap();
    // 5 秒是壳补“还在连接”的门槛：到那时核心已经给出了真实的等待时间和步骤。
    phone
        .wait_for(|view| {
            matches!(
                view.pairing,
                Some(PairingStatus::InProgress {
                    elapsed_ms: 5_000..,
                    stage: PairingStage::Connecting
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
    assert!(
        matches!(
            phone.dispatch(Action::Home {
                action: wonton_protocol::HomeAction::CancelPairing
            }),
            Err(CoreError::InvalidInput { .. })
        ),
        "没有正在进行的配对时取消该报输入错误"
    );
}
