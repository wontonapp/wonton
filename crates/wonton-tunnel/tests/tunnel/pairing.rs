//! 扫码配对：还没登记的设备经 `wonton/pair/0` 出示密钥，服务端的处理方决定登记还是拒绝。

use std::str::FromStr;
use std::sync::{Arc, Mutex};

use iroh::protocol::Router;
use iroh::{EndpointId, SecretKey};
use url::Url;
use wonton_tunnel::pair::{
    PAIR_ALPN, PairHandler, PairProtocol, PairResponse, PairingCode, PairingSecret, RejectReason,
    SECRET_LEN, pair,
};
use wonton_tunnel::{Allowlist, TunnelError, TunnelServer};

use crate::support::{Timeouts, loopback_endpoint};

/// 只认一个密钥，记下谁来配过。
#[derive(Debug)]
struct OneCode {
    secret: PairingSecret,
    paired: Mutex<Vec<(EndpointId, String)>>,
}

impl PairHandler for OneCode {
    fn pair(&self, device: EndpointId, secret: &PairingSecret, device_name: &str) -> PairResponse {
        if *secret != self.secret {
            return PairResponse::Rejected {
                reason: RejectReason::UnknownCode,
                message: "没发过这个码".to_owned(),
            };
        }
        self.paired
            .lock()
            .unwrap()
            .push((device, device_name.to_owned()));
        PairResponse::Paired
    }
}

fn code_for(server: &iroh::Endpoint, secret: PairingSecret) -> PairingCode {
    PairingCode {
        server_id: server.id(),
        relay_urls: Vec::new(),
        direct_addrs: server.bound_sockets(),
        base_domain: "wonton.localhost".to_owned(),
        core_url: Url::from_str("http://wonton.localhost/").unwrap(),
        secret,
    }
}

#[tokio::test]
async fn a_device_with_the_right_secret_is_paired_and_others_are_turned_away() {
    let server = loopback_endpoint(SecretKey::generate(), 0, Timeouts::Default).await;
    let handler = Arc::new(OneCode {
        secret: PairingSecret::from_bytes([7; SECRET_LEN]),
        paired: Mutex::new(Vec::new()),
    });
    let (tunnel, _listener) = TunnelServer::spawn_embedded_with_pairing(
        server.clone(),
        Allowlist::default(),
        PairProtocol::new(handler.clone()),
    );
    let phone_key = SecretKey::generate();
    let phone = loopback_endpoint(phone_key.clone(), 0, Timeouts::Default).await;

    let good = code_for(&server, handler.secret.clone());
    let reply = pair(&phone, &good, "Jiale 的 iPhone").await.unwrap();
    assert_eq!(reply, PairResponse::Paired);
    assert_eq!(
        *handler.paired.lock().unwrap(),
        vec![(phone_key.public(), "Jiale 的 iPhone".to_owned())],
        "登记的应是握手认证过的设备身份"
    );

    let wrong = code_for(&server, PairingSecret::from_bytes([8; SECRET_LEN]));
    let reply = pair(&phone, &wrong, "phone").await.unwrap();
    assert!(matches!(
        reply,
        PairResponse::Rejected {
            reason: RejectReason::UnknownCode,
            ..
        }
    ));
    tunnel.shutdown().await;
}

#[tokio::test]
async fn a_server_without_pairing_is_a_protocol_mismatch() {
    let server = loopback_endpoint(SecretKey::generate(), 0, Timeouts::Default).await;
    // 只挂隧道的 ALPN，像不支持配对的旧核心。
    let (tunnel, _listener) = TunnelServer::spawn_embedded(server.clone(), Allowlist::default());
    let phone = loopback_endpoint(SecretKey::generate(), 0, Timeouts::Default).await;
    let code = code_for(&server, PairingSecret::from_bytes([7; SECRET_LEN]));
    let error = pair(&phone, &code, "phone").await.unwrap_err();
    assert!(matches!(error, TunnelError::Protocol { .. }), "{error:?}");
    tunnel.shutdown().await;
}

#[tokio::test]
async fn a_garbled_request_gets_bad_request() {
    let server = loopback_endpoint(SecretKey::generate(), 0, Timeouts::Default).await;
    let handler = Arc::new(OneCode {
        secret: PairingSecret::from_bytes([7; SECRET_LEN]),
        paired: Mutex::new(Vec::new()),
    });
    let router = Router::builder(server.clone())
        .accept(PAIR_ALPN, PairProtocol::new(handler.clone()))
        .spawn();
    let phone = loopback_endpoint(SecretKey::generate(), 0, Timeouts::Default).await;
    let addr = code_for(&server, handler.secret.clone()).server_addr();
    let conn = phone.connect(addr, PAIR_ALPN).await.unwrap();
    let (mut send, mut recv) = conn.open_bi().await.unwrap();
    send.write_all(b"{\"type\": \"steal\"}").await.unwrap();
    send.finish().unwrap();
    let reply: PairResponse =
        serde_json::from_slice(&recv.read_to_end(4096).await.unwrap()).unwrap();
    assert!(matches!(
        reply,
        PairResponse::Rejected {
            reason: RejectReason::BadRequest,
            ..
        }
    ));
    assert!(handler.paired.lock().unwrap().is_empty());
    router.shutdown().await.unwrap();
}
