//! 配对测试共用的假服务端和手机：隧道入口只走回环地址，配对的处理方按发出去的码决定登记还是拒绝，
//! 登记就是把设备加进隧道的白名单；经隧道来的 HTTP 请求由一个模拟的核心回答。

pub(crate) use std::collections::{HashMap, HashSet};
pub(crate) use std::net::{Ipv4Addr, SocketAddr};
pub(crate) use std::ops::Deref;
pub(crate) use std::str::FromStr;
pub(crate) use std::sync::{Arc, Mutex};
pub(crate) use std::time::Duration;

pub(crate) use iroh::endpoint::presets;
pub(crate) use iroh::{Endpoint, EndpointId, SecretKey};
pub(crate) use serde_json::json;
pub(crate) use tokio::io::{AsyncReadExt, AsyncWriteExt};
pub(crate) use url::Url;
pub(crate) use wonton_client::{
    Action, ClientCore, CoreError, HomeStatus, HomeViewModel, Network, PairingFailure,
    PairingStatus, SecretString,
};
pub(crate) use wonton_protocol::{JoinOutcome, RoleDef};
pub(crate) use wonton_tunnel::pair::{
    PairHandler, PairProtocol, PairResponse, PairingCode, PairingSecret, RejectReason, SECRET_LEN,
};
pub(crate) use wonton_tunnel::{Allowlist, PeerStream, TunnelListener, TunnelServer};

/// 兜底预算，不是节奏：配对要另开 endpoint、建连再交换，繁忙时建连实测 16s+；连不上的服务端还要再确认一次
/// （两段 30 秒），取 90s，超过就是真的配不上。
pub(crate) const WAIT: Duration = Duration::from_secs(90);

/// 配上且连上了：核心把进展停在 `Paired`（壳离开配对页的类别），连接状态连着。
pub(crate) fn paired(view: &HomeViewModel) -> bool {
    matches!(view.pairing, Some(PairingStatus::Paired)) && view.status == HomeStatus::Connected
}

/// 服务端发过的码：密钥对应配对时要回的结果，`None` 表示配对成功。
#[derive(Debug)]
pub(crate) struct Codes {
    pub(crate) allowlist: Allowlist,
    pub(crate) server_id: EndpointId,
    pub(crate) direct_addrs: Vec<SocketAddr>,
    pub(crate) codes: Mutex<HashMap<String, Option<RejectReason>>>,
    pub(crate) refusal_message: Mutex<Option<String>>,
    /// 加入别人家的码：配对时回这个结果和角色，不登记设备。
    pub(crate) joins: Mutex<HashMap<String, (JoinOutcome, Option<RoleDef>)>>,
    pub(crate) names: Mutex<Vec<String>>,
    /// 设备经隧道要码（添加设备）时的请求体。
    pub(crate) invites: Mutex<Vec<String>>,
    /// 要码时回这个 HTTP 状态码的错误，为空时发一个新码。
    pub(crate) refuse: Mutex<Option<u16>>,
    pub(crate) reported_addrs: Mutex<Vec<SocketAddr>>,
    /// 接下来这么多个配对请求都拖住：每个都在处理里睡 [`SLOW_REPLY`] 才回，模拟慢中继上的回应。
    pub(crate) slow: Mutex<u32>,
    /// 已经见过的（码, 设备）。
    pub(crate) seen: Mutex<HashSet<(String, EndpointId)>>,
}

/// 比测试里的时限长得多：回应到之前就已经超时。
pub(crate) const SLOW_REPLY: Duration = Duration::from_secs(8);

impl Codes {
    /// 发一个码，配对时按 `outcome` 回应。
    pub(crate) fn issue(&self, outcome: Option<RejectReason>) -> String {
        self.issue_at(outcome, None)
    }

    /// 同上，直连地址换成 `addr`（拦在中间的回环转发）。
    pub(crate) fn issue_at(
        &self,
        outcome: Option<RejectReason>,
        addr: Option<SocketAddr>,
    ) -> String {
        let mut codes = self.codes.lock().unwrap();
        let secret =
            PairingSecret::from_bytes([u8::try_from(codes.len() + 1).unwrap(); SECRET_LEN]);
        codes.insert(secret.expose().to_owned(), outcome);
        PairingCode {
            server_id: self.server_id,
            relays: Vec::new(),
            direct_addrs: addr.map_or_else(|| self.direct_addrs.clone(), |addr| vec![addr]),
            base_domain: "wonton.localhost".to_owned(),
            core_url: Url::from_str("http://wonton.localhost:18480/").unwrap(),
            secret,
        }
        .to_uri()
    }
}

impl PairHandler for Codes {
    fn pair(&self, device: EndpointId, secret: &PairingSecret, device_name: &str) -> PairResponse {
        let key = (secret.expose().to_owned(), device);
        if let Some((outcome, role)) = self.joins.lock().unwrap().get(secret.expose()).cloned() {
            return PairResponse::Joining { outcome, role };
        }
        let first = self.seen.lock().unwrap().insert(key);
        // 登记先于回应，和真的核心一样：回应慢不代表登记慢，确认请求也不跟着一起慢。
        if first && matches!(self.codes.lock().unwrap().get(secret.expose()), Some(None)) {
            self.allowlist.allow(device);
            self.names.lock().unwrap().push(device_name.to_owned());
        }
        if *self.slow.lock().unwrap() > 0 {
            *self.slow.lock().unwrap() -= 1;
            std::thread::sleep(SLOW_REPLY);
        }
        match self.codes.lock().unwrap().get(secret.expose()) {
            Some(None) => PairResponse::Paired,
            Some(Some(reason)) => PairResponse::Rejected {
                reason: *reason,
                message: self
                    .refusal_message
                    .lock()
                    .unwrap()
                    .clone()
                    .unwrap_or_else(|| format!("{reason:?}")),
            },
            None => PairResponse::Rejected {
                reason: RejectReason::UnknownCode,
                message: "没发过".to_owned(),
            },
        }
    }
}

pub(crate) struct Home {
    pub(crate) server: TunnelServer,
    pub(crate) codes: Arc<Codes>,
}

impl Home {
    pub(crate) async fn start() -> Self {
        Self::start_at((Ipv4Addr::LOCALHOST, 0).into()).await
    }

    /// 起在给定地址上。同一端口重建时端口可能还被旧 endpoint 占着、或者刚空出来就被别的测试拿走，
    /// 所以反复 bind 到绑上为止，不先探再绑。
    pub(crate) async fn start_at(address: SocketAddr) -> Self {
        let key = SecretKey::generate();
        let deadline = tokio::time::Instant::now() + WAIT;
        let endpoint = loop {
            match Endpoint::builder(presets::Minimal)
                .secret_key(key.clone())
                .clear_ip_transports()
                .bind_addr(address)
                .unwrap()
                .bind()
                .await
            {
                Ok(endpoint) => break endpoint,
                Err(error) if crate::home::support::port_in_use(&error) => {
                    assert!(
                        tokio::time::Instant::now() < deadline,
                        "{address} 一直没有释放：{error}"
                    );
                    tokio::time::sleep(Duration::from_millis(1)).await;
                }
                Err(error) => panic!("{address} 起不来：{error}"),
            }
        };
        let allowlist = Allowlist::default();
        let codes = Arc::new(Codes {
            allowlist: allowlist.clone(),
            server_id: endpoint.id(),
            direct_addrs: endpoint.bound_sockets(),
            codes: Mutex::new(HashMap::new()),
            refusal_message: Mutex::new(None),
            joins: Mutex::new(HashMap::new()),
            names: Mutex::new(Vec::new()),
            invites: Mutex::new(Vec::new()),
            refuse: Mutex::new(None),
            reported_addrs: Mutex::new(Vec::new()),
            slow: Mutex::new(0),
            seen: Mutex::new(HashSet::new()),
        });
        let (server, listener) = TunnelServer::spawn_embedded_with_pairing(
            endpoint,
            allowlist,
            PairProtocol::new(codes.clone()),
        );
        tokio::spawn(fake_core(listener, codes.clone()));
        Self { server, codes }
    }

    pub(crate) fn code(&self, outcome: Option<RejectReason>) -> String {
        self.codes.issue(outcome)
    }

    /// 直连地址换成 `addr` 的码。
    pub(crate) fn code_at(&self, addr: SocketAddr) -> String {
        self.codes.issue_at(None, Some(addr))
    }

    /// 发一个加入别人家的码：配对时回 `outcome`，等批时带 `role`。
    pub(crate) fn join_code(&self, outcome: JoinOutcome, role: Option<RoleDef>) -> String {
        let code = self.code(None);
        let secret = PairingCode::parse(&code)
            .unwrap()
            .secret
            .expose()
            .to_owned();
        self.codes
            .joins
            .lock()
            .unwrap()
            .insert(secret, (outcome, role));
        code
    }
}

/// 模拟的核心：经隧道来的 `POST /_wonton/api/pairing-codes`（添加设备）发一个新码，或者按 `Codes::refuse` 回错误。
async fn fake_core(mut listener: TunnelListener, codes: Arc<Codes>) {
    while let Some(stream) = listener.accept().await {
        tokio::spawn(answer(stream, codes.clone()));
    }
}

async fn answer(mut stream: PeerStream, codes: Arc<Codes>) {
    let mut head = Vec::new();
    while !head.ends_with(b"\r\n\r\n") {
        let mut byte = [0u8; 1];
        if stream.read(&mut byte).await.unwrap_or(0) == 0 {
            return;
        }
        head.push(byte[0]);
    }
    let text = String::from_utf8_lossy(&head).to_ascii_lowercase();
    let length = text
        .lines()
        .find_map(|line| line.strip_prefix("content-length: "))
        .map_or(0, |value| value.trim().parse().unwrap());
    let mut body = vec![0u8; length];
    stream.read_exact(&mut body).await.unwrap();
    let (status, reply) = if text.starts_with("post /_wonton/api/pairing-codes ") {
        codes
            .invites
            .lock()
            .unwrap()
            .push(String::from_utf8(body).unwrap());
        let refused = *codes.refuse.lock().unwrap();
        match refused {
            Some(status) => (
                status,
                json!({"error": {"kind": "refused", "message": "不给"}}),
            ),
            None => (200, json!({"code": codes.issue(None), "expires_in": 600})),
        }
    } else if text.starts_with("get /_wonton/api/tunnel ") {
        (
            200,
            json!({
                "endpoint_id": codes.server_id,
                "relays": [],
                "direct_addrs": *codes.reported_addrs.lock().unwrap(),
            }),
        )
    } else if text.starts_with("get /after-restart ") {
        (200, json!({"device": stream.remote_id().to_string()}))
    } else {
        (
            404,
            json!({"error": {"kind": "not_found", "message": "没有这个接口"}}),
        )
    };
    let reply = reply.to_string();
    let response = format!(
        "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\
         connection: close\r\n\r\n{reply}",
        reply.len()
    );
    stream.write_all(response.as_bytes()).await.ok();
    stream.shutdown().await.ok();
}

pub(crate) struct Phone {
    core: Arc<ClientCore>,
}

impl Deref for Phone {
    type Target = ClientCore;

    fn deref(&self) -> &ClientCore {
        &self.core
    }
}

impl Phone {
    pub(crate) fn open(dir: &std::path::Path) -> Self {
        let core = ClientCore::with_network(dir.to_str().unwrap(), Network::DirectOnly).unwrap();
        Self { core }
    }

    pub(crate) fn pair(&self, code: &str) -> Result<(), CoreError> {
        self.dispatch(Action::Home {
            action: wonton_protocol::HomeAction::PairHome {
                code: SecretString::new(code),
                device_name: "Alice 的 iPhone".to_owned(),
            },
        })
    }

    pub(crate) async fn wait_for(
        &self,
        mut done: impl FnMut(&HomeViewModel) -> bool,
    ) -> HomeViewModel {
        let subscription = self.subscribe_home();
        let waiting = async {
            while let Some(view) = subscription.next().await {
                if done(&view) {
                    return view;
                }
            }
            panic!("订阅提前结束");
        };
        tokio::time::timeout(WAIT, waiting)
            .await
            .unwrap_or_else(|_| panic!("等待超时，当前：{:?}", self.home()))
    }

    /// 把摘要和独立诊断一起带入断言的失败输出，便于区分服务端先关和没读到请求。
    pub(crate) async fn pairing_failure(&self) -> (PairingFailure, String) {
        let view = self
            .wait_for(|view| matches!(view.pairing, Some(PairingStatus::Failed { .. })))
            .await;
        match view.pairing {
            Some(PairingStatus::Failed {
                reason,
                message,
                diagnostic,
            }) => (reason, format!("{message}\n{diagnostic:?}")),
            _ => unreachable!(),
        }
    }
}
