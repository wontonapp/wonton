//! 扫码配对：还没登记的设备经 `wonton/pair/0` 出示密钥，服务端的处理方决定登记还是拒绝。

use std::{
    str::FromStr,
    sync::{Arc, Mutex},
    time::Duration,
};

use iroh::endpoint::{Connection, VarInt};
use iroh::protocol::{AcceptError, ProtocolHandler, Router};
use iroh::{EndpointId, SecretKey};
use url::Url;
use wonton_tunnel::pair::{
    MAX_MESSAGE_LEN, PAIR_ALPN, PairHandler, PairProtocol, PairResponse, PairingCode,
    PairingSecret, RejectReason, SECRET_LEN, pair,
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
        relays: Vec::new(),
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
    let reply = pair(&phone, &good, "Alice 的 iPhone").await.unwrap();
    assert_eq!(reply, PairResponse::Paired);
    assert_eq!(
        *handler.paired.lock().unwrap(),
        vec![(phone_key.public(), "Alice 的 iPhone".to_owned())],
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

/// 回应写完就关连接、不发 FIN：设备侧看到的状态和“LINGER 超时后服务端先关、FIN 在负载下没能到设备”
/// 时一致——回应的字节已经在接收缓冲里，连接以 `done` 关闭。已经送达设备的决定必须算数。
#[derive(Debug)]
struct DiesAfterResponse {
    response: Vec<u8>,
}

impl ProtocolHandler for DiesAfterResponse {
    async fn accept(&self, conn: Connection) -> Result<(), AcceptError> {
        let (mut send, mut recv) = conn.accept_bi().await.expect("设备会开流");
        let _request = recv
            .read_to_end(MAX_MESSAGE_LEN)
            .await
            .expect("设备会写完请求");
        send.write_all(&self.response).await.expect("回应写得进流");
        // 让回应先于关闭帧到达设备；之后不再 finish，FIN 永远不来。
        tokio::time::sleep(Duration::from_millis(100)).await;
        conn.close(VarInt::from_u32(0), b"done");
        Ok(())
    }
}

#[tokio::test]
async fn a_rejection_the_device_already_received_survives_the_server_closing_first() {
    let server = loopback_endpoint(SecretKey::generate(), 0, Timeouts::Default).await;
    let response = serde_json::to_vec(&PairResponse::Rejected {
        reason: RejectReason::Used,
        message: "这个码已经用过".to_owned(),
    })
    .unwrap();
    let router = Router::builder(server.clone())
        .accept(PAIR_ALPN, DiesAfterResponse { response })
        .spawn();
    let phone = loopback_endpoint(SecretKey::generate(), 0, Timeouts::Default).await;
    let code = code_for(&server, PairingSecret::from_bytes([7; SECRET_LEN]));

    let reply = pair(&phone, &code, "phone").await;

    assert!(
        matches!(
            reply,
            Ok(PairResponse::Rejected {
                reason: RejectReason::Used,
                ..
            })
        ),
        "{reply:?}"
    );
    router.shutdown().await.unwrap();
}
