use std::net::{Ipv4Addr, SocketAddr};

use axum::{Json, Router, http::StatusCode};
use tokio::{
    io::copy_bidirectional,
    net::{TcpListener, TcpStream},
    sync::watch,
    task::JoinHandle,
};

/// 客户端经它连服务器的 TCP 中转，测试用它断网、让服务器拒收、再恢复。
///
/// 断开时关掉已有的连接，新连接一接上就关掉，客户端马上得到网络错误，不像
/// [`TestServer::freeze`](super::TestServer::freeze) 那样挂住。拒收时新连接转给本机一个对什么请求都回
/// 403 `M_FORBIDDEN` 的服务。恢复后照常转发。
pub struct Relay {
    url: String,
    mode: watch::Sender<Mode>,
    tasks: [JoinHandle<()>; 2],
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Forward,
    Cut,
    Reject,
}

impl Relay {
    /// 在本机的空闲端口上起中转，转发到 `target`（`http://主机:端口`）。
    pub async fn start(target: &str) -> Self {
        let target = target
            .strip_prefix("http://")
            .expect("只转发 http 地址")
            .trim_end_matches('/')
            .to_owned();
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .await
            .expect("中转监听本机端口");
        let url = format!("http://{}", listener.local_addr().expect("中转的地址"));
        let (refuser, refusing) = refuse().await;
        let (mode, current) = watch::channel(Mode::Forward);
        let targets = Targets {
            server: target,
            refuser: refuser.to_string(),
        };
        let forwarding = tokio::spawn(forward(listener, targets, current));
        Self {
            url,
            mode,
            tasks: [forwarding, refusing],
        }
    }

    /// 客户端连这个地址。
    pub fn url(&self) -> &str {
        &self.url
    }

    /// 断网：关掉已有的连接，之后的连接一接上就关掉。
    pub fn cut(&self) {
        self.mode.send_replace(Mode::Cut);
    }

    /// 服务器拒收：关掉已有的连接，之后的请求都得到 403 `M_FORBIDDEN`，SDK 当作不可恢复的错误。
    pub fn reject(&self) {
        self.mode.send_replace(Mode::Reject);
    }

    pub fn restore(&self) {
        self.mode.send_replace(Mode::Forward);
    }
}

impl Drop for Relay {
    fn drop(&mut self) {
        // 每条连接的转发在发送端随中转释放后自己结束。
        for task in &self.tasks {
            task.abort();
        }
    }
}

struct Targets {
    server: String,
    refuser: String,
}

async fn forward(listener: TcpListener, targets: Targets, mode: watch::Receiver<Mode>) {
    while let Ok((mut client, _)) = listener.accept().await {
        let current = *mode.borrow();
        let target = match current {
            Mode::Cut => continue,
            Mode::Forward => targets.server.clone(),
            Mode::Reject => targets.refuser.clone(),
        };
        let mut mode = mode.clone();
        tokio::spawn(async move {
            let Ok(mut server) = TcpStream::connect(&target).await else {
                return;
            };
            tokio::select! {
                _ = copy_bidirectional(&mut client, &mut server) => {}
                _ = mode.wait_for(|mode| *mode != current) => {}
            }
        });
    }
}

/// 本机一个对什么请求都回 403 `M_FORBIDDEN` 的服务。
async fn refuse() -> (SocketAddr, JoinHandle<()>) {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .await
        .expect("拒收服务监听本机端口");
    let address = listener.local_addr().expect("拒收服务的地址");
    let app = Router::new().fallback(|| async {
        let body =
            serde_json::json!({ "errcode": "M_FORBIDDEN", "error": "测试中转拒收了这个请求" });
        (StatusCode::FORBIDDEN, Json(body))
    });
    let task = tokio::spawn(async move { axum::serve(listener, app).await.expect("拒收服务") });
    (address, task)
}
