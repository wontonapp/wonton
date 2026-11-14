#[cfg(target_os = "linux")]
pub mod tls;

use std::collections::VecDeque;
use std::net::{IpAddr, SocketAddr};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::serve::ListenerExt;
use reqwest::{Client, Response};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;
use wonton_remote_mcp::{Config, ResolveFuture, Resolver, router};

pub struct Server {
    pub address: SocketAddr,
    pub connections: Arc<AtomicUsize>,
    task: JoinHandle<()>,
}

impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}

pub async fn serve(app: Router) -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let connections = Arc::new(AtomicUsize::new(0));
    let accepted = connections.clone();
    let listener = listener.tap_io(move |_| {
        accepted.fetch_add(1, Ordering::SeqCst);
    });
    let task = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    Server {
        address,
        connections,
        task,
    }
}

pub async fn connector(config: Config) -> Server {
    serve(router(config)).await
}

pub fn client() -> Client {
    Client::builder()
        .no_proxy()
        .no_gzip()
        .no_brotli()
        .no_deflate()
        .no_zstd()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap()
}

pub async fn forward(client: &Client, connector: SocketAddr, target: &str) -> Response {
    client
        .get(format!("http://{connector}/mcp"))
        .header("x-wonton-plugin", "network-contract")
        .header("x-wonton-remote-url", target)
        .header("x-wonton-remote-auth", "none")
        .header("x-wonton-remote-forward", "e30=")
        .send()
        .await
        .unwrap()
}

pub struct Answers {
    answers: Mutex<VecDeque<Vec<IpAddr>>>,
    last: Vec<IpAddr>,
    pub calls: AtomicUsize,
    pub hosts: Mutex<Vec<String>>,
}

impl Answers {
    pub fn new(answers: Vec<Vec<IpAddr>>) -> Arc<Self> {
        let last = answers.last().unwrap().clone();
        Arc::new(Self {
            answers: Mutex::new(answers.into()),
            last,
            calls: AtomicUsize::new(0),
            hosts: Mutex::new(Vec::new()),
        })
    }

    pub fn fixed(address: &str) -> Arc<Self> {
        Self::new(vec![vec![address.parse().unwrap()]])
    }
}

impl Resolver for Answers {
    fn resolve(&self, host: &str) -> ResolveFuture {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.hosts.lock().unwrap().push(host.to_owned());
        let answer = self
            .answers
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| self.last.clone());
        Box::pin(async move { Ok(answer) })
    }
}

pub struct RejectingProxy {
    pub server: Server,
    pub requests: Arc<Mutex<Vec<String>>>,
}

impl RejectingProxy {
    pub async fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let connections = Arc::new(AtomicUsize::new(0));
        let accepted = connections.clone();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let recorded = requests.clone();
        let task = tokio::spawn(async move {
            loop {
                let (mut socket, _) = listener.accept().await.unwrap();
                accepted.fetch_add(1, Ordering::SeqCst);
                let mut request = Vec::new();
                let mut chunk = [0; 1024];
                while !request.ends_with(b"\r\n\r\n") {
                    let count = socket.read(&mut chunk).await.unwrap();
                    if count == 0 {
                        break;
                    }
                    request.extend_from_slice(&chunk[..count]);
                    assert!(request.len() < 16 * 1024);
                }
                recorded
                    .lock()
                    .unwrap()
                    .push(String::from_utf8(request).unwrap());
                socket
                    .write_all(b"HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\n\r\n")
                    .await
                    .unwrap();
            }
        });
        Self {
            server: Server {
                address,
                connections,
                task,
            },
            requests,
        }
    }
}

pub struct Binary {
    child: Child,
    pub address: SocketAddr,
}

impl Binary {
    pub async fn start(extra_env: &[(&str, &str)]) -> Self {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let mut command = Self::command();
        command.env("WONTON_REMOTE_MCP_LISTEN", address.to_string());
        command.envs(extra_env.iter().copied());
        drop(listener);
        let mut binary = Self {
            child: command.spawn().unwrap(),
            address,
        };
        let client = client();
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        loop {
            assert!(
                binary.child.try_wait().unwrap().is_none(),
                "connector exited"
            );
            if client
                .get(format!("http://{address}/healthz"))
                .send()
                .await
                .is_ok_and(|response| response.status() == reqwest::StatusCode::NO_CONTENT)
            {
                break;
            }
            assert!(tokio::time::Instant::now() < deadline, "startup timed out");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        binary
    }

    pub fn command() -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_wonton-remote-mcp"));
        command
            .env_remove("HTTPS_PROXY")
            .env_remove("https_proxy")
            .env_remove("HTTP_PROXY")
            .env_remove("http_proxy")
            .env_remove("ALL_PROXY")
            .env_remove("all_proxy")
            .env_remove("WONTON_REMOTE_MCP_BLOCKED_NETWORKS")
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        command
    }
}

impl Drop for Binary {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

pub async fn isolated_test(name: &str, variables: &[(&str, &str)]) {
    let mut command = Command::new(std::env::current_exe().unwrap());
    for variable in [
        "HTTPS_PROXY",
        "https_proxy",
        "HTTP_PROXY",
        "http_proxy",
        "ALL_PROXY",
        "all_proxy",
        "WONTON_REMOTE_MCP_BLOCKED_NETWORKS",
    ] {
        command.env_remove(variable);
    }
    command
        .args(["--exact", name, "--nocapture"])
        .env("WONTON_REMOTE_MCP_TEST_ENV", name)
        .envs(variables.iter().copied())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().unwrap();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        if child.try_wait().unwrap().is_some() {
            let output = child.wait_with_output().unwrap();
            assert!(
                output.status.success(),
                "{}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            return;
        }
        if tokio::time::Instant::now() >= deadline {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("isolated network test timed out");
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}
