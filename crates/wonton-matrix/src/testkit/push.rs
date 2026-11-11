//! 测试用的推送网关：扮演核心的推送代理，Tuwunel 经出站代理把通知发到这里，和产品里一样（ADR-0003 的
//! “接口：推送”）。记下收到的通知，可以让它对指定的 pushkey 回 `rejected`。

use std::collections::BTreeSet;
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::{Arc, Mutex};

use axum::extract::State;
use axum::http::HeaderMap;
use axum::routing::post;
use axum::{Json, Router};
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio::sync::watch;

/// 收到的一次通知：`Host` 头和 `notification` 对象。
#[derive(Clone, Debug)]
pub struct Received {
    pub host: String,
    pub notification: Value,
}

#[derive(Clone, Default)]
struct Shared {
    received: Arc<Mutex<Vec<Received>>>,
    rejecting: Arc<Mutex<BTreeSet<String>>>,
    count: Arc<watch::Sender<usize>>,
}

pub struct PushGateway {
    addr: SocketAddr,
    shared: Shared,
}

impl PushGateway {
    /// pusher 的地址。这个域名解析不到：Tuwunel 只能经 [`Self::tuwunel_proxy`] 配的出站代理到达这里。
    pub const URL: &'static str = "http://push.wonton.internal/_matrix/push/v1/notify";

    /// Linux 上 Tuwunel 用宿主的网络，经回环地址连过来；别处经 `host.docker.internal`（宿主网关）连过来，要听所有
    /// 地址（见 [`TestServer::with_push_gateway`](super::TestServer::with_push_gateway)）。
    pub async fn start() -> Self {
        let shared = Shared::default();
        let app = Router::new()
            .route("/_matrix/push/v1/notify", post(notify))
            .with_state(shared.clone());
        let listen = if cfg!(target_os = "linux") {
            Ipv4Addr::LOCALHOST
        } else {
            Ipv4Addr::UNSPECIFIED
        };
        let listener = TcpListener::bind((listen, 0)).await.expect("推送网关监听");
        let addr = listener.local_addr().expect("推送网关地址");
        tokio::spawn(async move { axum::serve(listener, app).await.expect("推送网关") });
        Self { addr, shared }
    }

    /// Tuwunel 的 `TUWUNEL_PROXY`：只代理推送代理的域名，代理地址是这里。
    pub fn tuwunel_proxy(&self) -> String {
        let host = if cfg!(target_os = "linux") {
            "127.0.0.1"
        } else {
            "host.docker.internal"
        };
        format!(
            r#"{{by_domain=[{{url="http://{host}:{}",include=["push.wonton.internal"]}}]}}"#,
            self.addr.port()
        )
    }

    /// 之后对这个 pushkey 回 `rejected`。
    pub fn reject(&self, pushkey: &str) {
        self.shared
            .rejecting
            .lock()
            .unwrap()
            .insert(pushkey.to_owned());
    }

    /// 到目前为止收到的通知。
    pub fn received(&self) -> Vec<Received> {
        self.shared.received.lock().unwrap().clone()
    }

    /// 等一条满足条件的通知（包括已经收到的），超时就失败。
    pub async fn wait_for(&self, what: &str, mut check: impl FnMut(&Received) -> bool) -> Received {
        let mut count = self.shared.count.subscribe();
        let waiting = async {
            loop {
                if let Some(found) = self.received().into_iter().find(|r| check(r)) {
                    return found;
                }
                count.changed().await.expect("推送网关已停止");
            }
        };
        match tokio::time::timeout(super::WAIT, waiting).await {
            Ok(found) => found,
            Err(_) => panic!(
                "{what}：{:?} 内没有等到，收到的通知：{:#?}",
                super::WAIT,
                self.received()
            ),
        }
    }
}

async fn notify(
    State(shared): State<Shared>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Json<Value> {
    let host = headers
        .get("host")
        .and_then(|host| host.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    let notification = body["notification"].clone();
    let rejecting = shared.rejecting.lock().unwrap().clone();
    let rejected: Vec<&str> = notification["devices"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|device| device["pushkey"].as_str())
        .filter(|pushkey| rejecting.contains(*pushkey))
        .collect();
    let response = json!({ "rejected": rejected });
    let total = {
        let mut received = shared.received.lock().unwrap();
        received.push(Received { host, notification });
        received.len()
    };
    shared.count.send_replace(total);
    Json(response)
}
