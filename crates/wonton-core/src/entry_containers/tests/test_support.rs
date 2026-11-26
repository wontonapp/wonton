//! 共用夹具：假 Docker 装置、设置和文件帮手、假 HAProxy 管理端口、联邦注册设置。

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::{Value, json};

use super::super::{EntryContainers, EntryError};
use crate::entry_settings::EntrySettings;
use crate::plugin::fake_docker::FakeDocker;
use crate::plugin::names::Instance;

const CORE: &str = "wonton-core";
pub(super) const PROJECT: &str = "wonton";

pub(super) fn settings() -> EntrySettings {
    EntrySettings {
        domain: "k3f9qzab.example.test".to_owned(),
        service_url: None,
        service_ca: None,
        relay_ca: None,
        registration: None,
        acme_directory: None,
        acme_ca: None,
        acme_resolvers: Vec::new(),
        acme_email: None,
        acme_accept_terms: false,
        acme_network: "bridge".to_owned(),
        dns_provider: None,
        dns_env_file: None,
        acme_propagation_disable_ans: false,
        external_certificate: true,
    }
}

/// 主人在 App 里配了自己的域名：“不要名字”的安装没有设置文件，只有这份域名设置。
pub(super) fn app_domain(rig: &Rig, suffix: &str) {
    std::fs::write(
        rig.dir.path().join("entry/domain.json"),
        serde_json::to_vec(&json!({"suffix": suffix, "terms_accepted": true})).unwrap(),
    )
    .unwrap();
}

/// 发布一张覆盖这些名字的证书，和签发成功落盘的 `site.pem` 一样。
pub(super) fn publish_cert(rig: &Rig, names: &[&str]) {
    let names: Vec<String> = names.iter().map(|name| (*name).to_owned()).collect();
    let cert = rcgen::generate_simple_self_signed(names).unwrap();
    std::fs::write(rig.dir.path().join("entry/certs/site.pem"), cert.cert.pem()).unwrap();
}

/// 命令行 `-f`（或 frpc 的 `-c`）指到的配置文件：容器里的 `/config` 就是 `entry/`。
pub(super) fn command_file(rig: &Rig, container: &str) -> std::path::PathBuf {
    let created = rig.created(container);
    let arguments = created["Cmd"].as_array().unwrap();
    let position = arguments
        .iter()
        .position(|argument| argument == "-f" || argument == "-c")
        .unwrap();
    let name = arguments[position + 1]
        .as_str()
        .unwrap()
        .strip_prefix("/config/")
        .unwrap();
    rig.dir.path().join("entry").join(name)
}

/// 容器挂进去的每个源路径都得真的在：缺了 Docker 会在宿主侧建成目录，haproxy 起不来。
pub(super) fn assert_mounts_exist(rig: &Rig, container: &str) {
    let created = rig.created(container);
    for bind in created["HostConfig"]["Binds"].as_array().unwrap() {
        let source = bind.as_str().unwrap().split(':').next().unwrap();
        assert!(std::path::Path::new(source).exists(), "{source} 不在");
    }
}

pub(super) fn config(rig: &Rig) -> String {
    std::fs::read_to_string(rig.dir.path().join("entry/haproxy.cfg")).unwrap_or_default()
}
/// 公网 crt-list 里列的名字。
pub(super) fn served_names(rig: &Rig) -> Vec<String> {
    std::fs::read_to_string(rig.dir.path().join("entry/certs/public.crt-list"))
        .unwrap_or_default()
        .split_whitespace()
        .map(str::to_owned)
        .collect()
}

pub(super) struct Rig {
    pub(super) docker: FakeDocker,
    pub(super) containers: EntryContainers,
    pub(super) dir: tempfile::TempDir,
    /// 管理端口被连了几次：每连一次就是一次 reload。
    reloads: std::sync::Arc<std::sync::atomic::AtomicUsize>,
}

impl Rig {
    /// 核心在自己的 compose 项目里；`wanted` 里的网络先建好（全新安装时 edge、relay 还不存在）。
    pub(super) async fn new() -> Self {
        Self::with_networks(&["clients", "edge", "relay"]).await
    }

    async fn with_networks(networks: &[&str]) -> Self {
        let (docker, engine) = FakeDocker::start().await;
        docker.add_stranger_container(
            CORE,
            json!({"Labels": {"com.docker.compose.project": PROJECT}}),
        );
        for network in networks {
            docker.add_unlabeled_network(&format!("{PROJECT}_{network}"));
        }
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("entry/certs")).unwrap();
        std::fs::write(dir.path().join("entry/certs/site.pem"), "pem").unwrap();
        let (reload_addr, reloads) = counting_master().await;
        let containers = EntryContainers::new(
            engine,
            dir.path().to_owned(),
            dir.path().join("data"),
            Instance::default(),
            Some(CORE.to_owned()),
        )
        .with_reload_addr(reload_addr);
        Self {
            docker,
            containers,
            dir,
            reloads,
        }
    }

    pub(super) fn reloads(&self) -> usize {
        self.reloads.load(std::sync::atomic::Ordering::SeqCst)
    }

    pub(super) async fn reconcile(
        &self,
        settings: Option<&EntrySettings>,
    ) -> Result<(), EntryError> {
        self.containers.reconcile(settings).await
    }

    pub(super) fn created(&self, name: &str) -> Value {
        self.docker.created(name).unwrap_or_default()
    }

    /// 核心自己也在容器的表里，这里只看它以外的。
    pub(super) fn others(&self) -> Vec<String> {
        self.docker
            .containers()
            .into_iter()
            .filter(|name| name != CORE)
            .collect()
    }

    pub(super) fn writes_for(&self, needle: &str) -> Vec<String> {
        self.docker
            .writes()
            .into_iter()
            .filter(|write| write.contains(needle))
            .collect()
    }
}

async fn read_command(socket: &mut tokio::net::TcpStream) {
    use tokio::io::AsyncReadExt;
    let mut byte = [0u8; 1];
    loop {
        if socket.read_exact(&mut byte).await.is_err() || byte[0] == b'\n' {
            break;
        }
    }
}

/// 假 HAProxy 主进程的管理端口：收一条命令，回一段固定回应，返回命令原文。
pub(super) async fn master_cli(reply: &'static str) -> (String, tokio::task::JoinHandle<String>) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio_listener().await;
    let addr = listener.local_addr().unwrap().to_string();
    let handle = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut command = Vec::new();
        let mut byte = [0u8; 1];
        loop {
            socket.read_exact(&mut byte).await.unwrap();
            command.push(byte[0]);
            if byte[0] == b'\n' {
                break;
            }
        }
        socket.write_all(reply.as_bytes()).await.unwrap();
        String::from_utf8(command).unwrap()
    });
    (addr, handle)
}

/// 每次连接都回同一段的假主进程：要在 reconcile 路径上重载的用例用。
/// 数着连接的假主进程：每连一次记一次，回成功。
/// 第一条连接回拒绝（带启动日志），之后回成功；返回连了几次。
pub(super) async fn failing_once_master() -> (String, std::sync::Arc<std::sync::atomic::AtomicUsize>)
{
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio_listener().await;
    let addr = listener.local_addr().unwrap().to_string();
    let attempts = Arc::new(AtomicUsize::new(0));
    let seen = attempts.clone();
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            let seen = seen.clone();
            tokio::spawn(async move {
                let mut byte = [0u8; 1];
                loop {
                    if socket.read_exact(&mut byte).await.is_err() || byte[0] == b'\n' {
                        break;
                    }
                }
                let first = seen.fetch_add(1, Ordering::SeqCst) == 0;
                let reply: &[u8] = if first {
                    b"Success=0\n--\nconfig error\n"
                } else {
                    b"Success=1\n"
                };
                let _ = socket.write_all(reply).await;
            });
        }
    });
    (addr, attempts)
}

async fn counting_master() -> (String, std::sync::Arc<std::sync::atomic::AtomicUsize>) {
    use std::sync::Arc;
    use std::sync::atomic::AtomicUsize;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio_listener().await;
    let addr = listener.local_addr().unwrap().to_string();
    let count = Arc::new(AtomicUsize::new(0));
    let seen = count.clone();
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            seen.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let mut byte = [0u8; 1];
            loop {
                if socket.read_exact(&mut byte).await.is_err() || byte[0] == b'\n' {
                    break;
                }
            }
            if socket.write_all(b"Success=1\n").await.is_err() {
                return;
            }
        }
    });
    (addr, count)
}

pub(super) async fn master_forever(reply: &'static str) -> String {
    use tokio::io::AsyncWriteExt;
    let listener = tokio_listener().await;
    let addr = listener.local_addr().unwrap().to_string();
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            read_command(&mut socket).await;
            if socket.write_all(reply.as_bytes()).await.is_err() {
                return;
            }
            // 真的 master 返状态后关连接：读的一方靠 EOF 收尾，失败用例也不用干等截止。
            drop(socket);
        }
    });
    addr
}

/// 按顺序回应的假主进程：每次连接取一条回应，返回到管理端口的地址和已收的连接数。
pub(super) async fn master_sequence(replies: Vec<&'static str>) -> (String, Arc<AtomicUsize>) {
    use tokio::io::AsyncWriteExt;
    let listener = tokio_listener().await;
    let addr = listener.local_addr().unwrap().to_string();
    let hits = Arc::new(AtomicUsize::new(0));
    let counter = hits.clone();
    tokio::spawn(async move {
        for reply in replies {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            read_command(&mut socket).await;
            counter.fetch_add(1, Ordering::SeqCst);
            if socket.write_all(reply.as_bytes()).await.is_err() {
                return;
            }
            drop(socket);
        }
    });
    (addr, hits)
}

/// 第一条连接读了命令后既不回也不关，后续连接按顺序回 `replies`。
pub(super) async fn master_silent_then(replies: Vec<&'static str>) -> (String, Arc<AtomicUsize>) {
    use tokio::io::AsyncWriteExt;
    let listener = tokio_listener().await;
    let addr = listener.local_addr().unwrap().to_string();
    let hits = Arc::new(AtomicUsize::new(0));
    let counter = hits.clone();
    tokio::spawn(async move {
        let mut silent = Vec::new();
        for index in 0..=replies.len() {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            read_command(&mut socket).await;
            counter.fetch_add(1, Ordering::SeqCst);
            if index == 0 {
                // 不说话也不关，读完命令就搁着。
                silent.push(socket);
                continue;
            }
            if socket
                .write_all(replies[index - 1].as_bytes())
                .await
                .is_err()
            {
                return;
            }
            drop(socket);
        }
    });
    (addr, hits)
}

async fn tokio_listener() -> tokio::net::TcpListener {
    tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap()
}

pub(super) fn federation_registration() -> crate::home_service::Registration {
    serde_json::from_value(json!({
        "home": "k3f9qzab",
        "domain": "k3f9qzab.example.test",
        "lan_acme_dns": {"username": "u", "password": "p", "subdomain": "s", "fulldomain": "f"},
        "acme_dns": {"username": "u", "password": "p", "subdomain": "s", "fulldomain": "f"},
        "suspended": false,
        "acme_dns_url": "https://auth.example.test/update",
        "frp_token": "token",
        "relay": {"host": "relay.example.test", "port": 443, "server_name": "relay.example.test"}
    }))
    .unwrap()
}
