//! 测试里的家：一次性的 Tuwunel，加一个模拟的核心（签发 Matrix 登录令牌，把 `matrix.<基础域名>` 转给 Tuwunel），
//! 前面接真实的隧道服务端。手机是真实的客户端核心，经隧道和本地代理访问，和 App 一样。

use std::net::{Ipv4Addr, SocketAddr};
use std::ops::Deref;
use std::str::FromStr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use iroh::endpoint::presets;
use iroh::{Endpoint, EndpointId, SecretKey};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinHandle;
use wonton_client::{
    Action, ChatAccount, ChatSync, ChatViewModel, ClientCore, Network, TimelineDiff, TimelineItem,
    TimelineSubscription,
};
use wonton_matrix::MatrixSession;
use wonton_matrix::testkit::{Issuer, PushGateway, TestServer, random_hex};
use wonton_tunnel::{Allowlist, TunnelServer};

pub const WAIT: Duration = Duration::from_secs(60);
const BASE_DOMAIN: &str = "wonton.localhost";
/// 和 deploy/dev/matrix/tuwunel.toml 的 server_name 一致。
pub const OWNER: &str = "@owner:wonton.localhost";

pub struct Home {
    pub matrix: TestServer,
    /// 扮演核心的推送代理：Tuwunel 经出站代理把通知发到这里，和产品里一样。
    pub push: PushGateway,
    core: Arc<Core>,
    tunnel: TunnelServer,
    _core: JoinHandle<()>,
}

/// 模拟的核心记下的东西。
struct Core {
    tuwunel: String,
    issuer: Issuer,
    login_tokens: AtomicUsize,
    /// 登记推送时回给设备的 pushkey。
    pushkey: String,
    /// 推送登记接口收到的请求：方法和请求体（`DELETE` 没有请求体，是 `Null`）。
    push_requests: Mutex<Vec<(String, Value)>>,
}

impl Home {
    pub async fn start(devices: &[EndpointId]) -> Self {
        let push = PushGateway::start().await;
        let matrix = TestServer::with_push_gateway(&push).await;
        let core = Arc::new(Core {
            tuwunel: matrix.url.trim_start_matches("http://").to_owned(),
            issuer: matrix.issuer.clone(),
            login_tokens: AtomicUsize::new(0),
            pushkey: random_hex(32),
            push_requests: Mutex::default(),
        });
        let (addr, task) = fake_core(core.clone()).await;
        let endpoint = loopback_endpoint(SecretKey::generate()).await;
        let tunnel = TunnelServer::spawn(endpoint, Allowlist::new(devices.iter().copied()), addr);
        Self {
            matrix,
            push,
            core,
            tunnel,
            _core: task,
        }
    }

    /// 登记推送时核心回给设备的 pushkey。
    pub fn pushkey(&self) -> &str {
        &self.core.pushkey
    }

    /// 推送登记接口收到的请求。
    pub fn push_requests(&self) -> Vec<(String, Value)> {
        self.core.push_requests.lock().unwrap().clone()
    }

    pub fn ticket(&self) -> String {
        let id = self.tunnel.endpoint().id();
        let addr = self.tunnel.endpoint().bound_sockets()[0];
        format!(
            r#"{{"server": {{"endpoint_id": "{id}", "direct_addrs": ["{addr}"]}},
                "base_domain": "{BASE_DOMAIN}", "core_url": "http://{BASE_DOMAIN}"}}"#
        )
    }

    /// 核心签发过几次登录令牌。
    pub fn login_tokens(&self) -> usize {
        self.core.login_tokens.load(Ordering::SeqCst)
    }

    /// 一个 Bot 账号：直接连 Tuwunel（Bot 跑在家里的服务器上，不经隧道），设好显示名并开始同步。
    pub async fn bot(&self, localpart: &str, display_name: &str) -> MatrixSession {
        let bot = self.matrix.login(localpart, display_name).await;
        bot.set_display_name(display_name)
            .await
            .expect("Bot 设置显示名");
        bot.start_sync().await.expect("Bot 开始同步");
        bot
    }
}

/// 模拟的核心：`POST /_wonton/matrix/login-token` 签发主人的登录令牌（字段同核心的 `LoginToken`），
/// `PUT`、`DELETE /_wonton/push/registration` 记下推送登记（响应同核心的推送登记接口），`matrix.<基础域名>` 上的
/// 请求原样转给 Tuwunel。本地代理每个请求一条连接，读完请求头就能分流。
async fn fake_core(core: Arc<Core>) -> (SocketAddr, JoinHandle<()>) {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let addr = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        loop {
            let (tcp, _) = listener.accept().await.unwrap();
            tokio::spawn(serve(tcp, core.clone()));
        }
    });
    (addr, task)
}

async fn serve(mut tcp: TcpStream, core: Arc<Core>) {
    let mut head = Vec::new();
    while !head.ends_with(b"\r\n\r\n") {
        let mut byte = [0u8; 1];
        if tcp.read(&mut byte).await.unwrap_or(0) == 0 {
            return;
        }
        head.push(byte[0]);
    }
    let text = String::from_utf8_lossy(&head).to_ascii_lowercase();
    if text.contains(&format!("\r\nhost: matrix.{BASE_DOMAIN}")) {
        let mut upstream = TcpStream::connect(core.tuwunel.as_str()).await.unwrap();
        upstream.write_all(&head).await.unwrap();
        tokio::io::copy_bidirectional(&mut tcp, &mut upstream)
            .await
            .ok();
        return;
    }
    let length = text
        .lines()
        .find_map(|line| line.strip_prefix("content-length:"))
        .and_then(|value| value.trim().parse().ok())
        .unwrap_or(0);
    let mut body = vec![0u8; length];
    if tcp.read_exact(&mut body).await.is_err() {
        return;
    }
    let response = match respond(&core, &text, &body) {
        Some(body) => format!(
            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\
             connection: close\r\n\r\n{body}",
            body.len()
        ),
        None => {
            "HTTP/1.1 404 Not Found\r\ncontent-length: 0\r\nconnection: close\r\n\r\n".to_owned()
        }
    };
    tcp.write_all(response.as_bytes()).await.ok();
    tcp.shutdown().await.ok();
}

/// 核心自己的接口；不认识的请求回 `None`（404）。
fn respond(core: &Core, head: &str, body: &[u8]) -> Option<String> {
    let push = |method: &str, body: Value| {
        core.push_requests
            .lock()
            .unwrap()
            .push((method.to_owned(), body));
    };
    let response = if head.starts_with("post /_wonton/matrix/login-token ") {
        core.login_tokens.fetch_add(1, Ordering::SeqCst);
        json!({
            "homeserver_url": format!("http://matrix.{BASE_DOMAIN}"),
            "server_name": BASE_DOMAIN,
            "user_id": OWNER,
            "login_type": "org.matrix.login.jwt",
            "token": core.issuer.login_token("owner"),
            "expires_in": 120,
        })
    } else if head.starts_with("put /_wonton/push/registration ") {
        push("PUT", serde_json::from_slice(body).unwrap_or(Value::Null));
        json!({"pushkey": core.pushkey, "gateway_url": PushGateway::URL})
    } else if head.starts_with("delete /_wonton/push/registration ") {
        push("DELETE", Value::Null);
        json!({"removed": true})
    } else {
        return None;
    };
    Some(response.to_string())
}

async fn loopback_endpoint(key: SecretKey) -> Endpoint {
    Endpoint::builder(presets::Minimal)
        .secret_key(key)
        .clear_ip_transports()
        .bind_addr((Ipv4Addr::LOCALHOST, 0))
        .unwrap()
        .bind()
        .await
        .unwrap()
}

/// 测试里的手机：一个客户端核心，连同它的数据目录。
pub struct Phone {
    core: Arc<ClientCore>,
}

impl Deref for Phone {
    type Target = ClientCore;

    fn deref(&self) -> &ClientCore {
        &self.core
    }
}

impl Phone {
    pub fn open(dir: &std::path::Path) -> Self {
        let core = ClientCore::with_network(dir.to_str().unwrap(), Network::DirectOnly).unwrap();
        Self { core }
    }

    pub fn device_id(&self) -> EndpointId {
        EndpointId::from_str(&self.home().device_id).unwrap()
    }

    pub fn connect(&self, home: &Home) {
        self.dispatch(Action::ConnectHome {
            ticket: home.ticket(),
        })
        .unwrap();
    }

    /// 等聊天状态满足条件。
    pub async fn wait_chat<R>(
        &self,
        what: &str,
        mut check: impl FnMut(&ChatViewModel) -> Option<R>,
    ) -> R {
        let subscription = self.subscribe_chat();
        let mut last = None;
        let waiting = async {
            while let Some(view) = subscription.next().await {
                if let Some(found) = check(&view) {
                    return found;
                }
                last = Some(view);
            }
            panic!("订阅提前结束");
        };
        match tokio::time::timeout(WAIT, waiting).await {
            Ok(found) => found,
            Err(_) => panic!("{what}：{WAIT:?} 内没有等到，当前：{:#?}", self.chat()),
        }
    }

    /// 等登录并同步起来，返回用户 ID 和设备 ID。
    pub async fn signed_in(&self) -> (String, String) {
        self.wait_chat("登录并开始同步", |view| {
            match (&view.account, &view.sync) {
                (ChatAccount::SignedIn { user_id, device_id }, ChatSync::Running) => {
                    Some((user_id.clone(), device_id.clone()))
                }
                _ => None,
            }
        })
        .await
    }
}

/// 壳那一侧维护的时间线：按顺序应用每一批增量。
pub struct Timeline {
    pub items: Vec<TimelineItem>,
    subscription: Arc<TimelineSubscription>,
}

impl Timeline {
    pub async fn open(phone: &Phone, room_id: &str) -> Self {
        let subscription = phone
            .subscribe_timeline(room_id.to_owned())
            .await
            .expect("订阅时间线");
        Self {
            items: Vec::new(),
            subscription,
        }
    }

    /// 应用增量，直到列表里有满足条件的条目。
    pub async fn until<R>(
        &mut self,
        what: &str,
        mut check: impl FnMut(&TimelineItem) -> Option<R>,
    ) -> R {
        let waiting = async {
            loop {
                if let Some(found) = self.items.iter().find_map(&mut check) {
                    return found;
                }
                let update = self.subscription.next().await.expect("时间线订阅提前结束");
                for diff in update.diffs {
                    apply(&mut self.items, diff);
                }
            }
        };
        match tokio::time::timeout(WAIT, waiting).await {
            Ok(found) => found,
            Err(_) => panic!("{what}：{WAIT:?} 内没有等到"),
        }
    }
}

fn apply(items: &mut Vec<TimelineItem>, diff: TimelineDiff) {
    match diff {
        TimelineDiff::Append { items: more } => items.extend(more),
        TimelineDiff::Clear => items.clear(),
        TimelineDiff::PushFront { item } => items.insert(0, item),
        TimelineDiff::PushBack { item } => items.push(item),
        TimelineDiff::PopFront => {
            items.remove(0);
        }
        TimelineDiff::PopBack => {
            items.pop();
        }
        TimelineDiff::Insert { index, item } => items.insert(index as usize, item),
        TimelineDiff::Set { index, item } => items[index as usize] = item,
        TimelineDiff::Remove { index } => {
            items.remove(index as usize);
        }
        TimelineDiff::Truncate { length } => items.truncate(length as usize),
        TimelineDiff::Reset { items: all } => *items = all,
    }
}
