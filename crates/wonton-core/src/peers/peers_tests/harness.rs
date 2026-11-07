//! 起测试核心：设备表、隧道 endpoint、配对协议和联邦入站都在这里装配。

use super::*;
use crate::members::Members;

impl TestCore {
    /// 起一台预置了待处理请求（接收时刻都很旧）和这一小时已收条数的核心。
    pub(super) async fn start_seeded(
        lang: &str,
        name: &str,
        server_name: &str,
        pending: Vec<usize>,
        recent: usize,
    ) -> Self {
        let notices = Arc::new(Notices::default());
        let dir = tempfile::tempdir().unwrap();
        let requests: Vec<PeerRequest> = pending
            .into_iter()
            .map(|index| PeerRequest {
                id: format!("p{index}"),
                endpoint_id: SecretKey::generate().public(),
                server_name: format!("p{index}.example.internal"),
                owner_user_id: format!("@owner:p{index}.example.internal"),
                owner_display_name: "Pending".to_owned(),
                relays: Vec::new(),
                addrs: Vec::new(),
                received_at_ms: 1,
            })
            .collect();
        std::fs::create_dir_all(dir.path().join("core")).unwrap();
        std::fs::write(
            dir.path().join("core/peers.json"),
            serde_json::to_string(&super::super::Stored {
                requests,
                received: vec![super::super::now_ms(); recent],
                ..Default::default()
            })
            .unwrap(),
        )
        .unwrap();
        let devices = Arc::new(Devices::open(dir.path().join("devices.json")).unwrap());
        let members = Arc::new(Members::open(dir.path().join("members.json")).unwrap());
        let key = SecretKey::generate();
        let endpoint = loopback_endpoint(key.clone()).await;
        let tunnel = Tunnel::new(
            devices,
            members,
            Arc::new(|| crate::plugin::languages::Languages::from_header(None)),
            endpoint,
            0,
            RelayTable::builtin_n0(),
            watch::channel(BTreeSet::from(["127.0.0.1".parse().unwrap()])).1,
        );
        let owner_name = Arc::new(Mutex::new(name.to_owned()));
        let own = OwnIdentity {
            server_name: server_name.to_owned(),
            owner_user_id: format!("@owner:{server_name}"),
        };
        let peers = Peers::open(
            dir.path().join("core/peers.json"),
            own.clone(),
            tunnel,
            Some(notices.clone()),
            Arc::new({
                let owner_name = owner_name.clone();
                move || owner_name.lock().unwrap().clone()
            }),
            {
                let lang = lang.to_owned();
                Arc::new(move || Languages::from_header(Some(&lang)))
            },
        )
        .unwrap();
        let paired_names = peers.paired_names();
        let upstream = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let federation = FederationProtocol::new(
            peers.server_allowlist().clone(),
            upstream.local_addr().unwrap(),
        );
        let (_server, _listener) = wonton_tunnel::TunnelServer::spawn_embedded_with_pairing_and(
            peers.tunnel().endpoint.clone(),
            peers.tunnel().devices.allowlist().clone(),
            wonton_tunnel::pair::PairProtocol::new(peers.tunnel().pairing.clone()),
            [
                (
                    PEER_PAIR_ALPN,
                    Box::new(wonton_tunnel::peer_pair::PeerPairProtocol::new(Arc::new(
                        super::super::pairing::PeerPairing(peers.clone()),
                    ))) as Box<dyn iroh::protocol::DynProtocolHandler>,
                ),
                (
                    FEDERATION_ALPN,
                    Box::new(federation) as Box<dyn iroh::protocol::DynProtocolHandler>,
                ),
            ],
        );
        Self {
            peers,
            key,
            own,
            owner_name,
            notices,
            paired_names,
            _server: Some(_server),
            _upstream: upstream,
            forwarder: None,
            states: None,
            dir,
        }
    }
}
