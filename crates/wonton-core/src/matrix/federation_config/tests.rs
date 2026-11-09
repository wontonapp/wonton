use std::collections::BTreeSet;
use std::process::Command;
use std::time::Duration;

use hyper::{Method, StatusCode};

use super::{allowlist_toml, files::regex_escape};
use crate::http::{HttpClient, RequestBody};
use crate::plugin::docker::Engine;

/// Tuwunel：本地按标签拉；CI 经 `WONTON_TEST_IMAGES` 指到镜像仓库副本（wonton-test-images）。
pub(super) fn image() -> String {
    wonton_test_images::image("ghcr.io/matrix-construct/tuwunel:v1.9.4")
}
const WAIT: Duration = Duration::from_secs(15);

/// 真实 Tuwunel（v1.9.4）：两份配置里的名单生效，原地重写第二份后发 SIGUSR1，
/// 名单外的邀请被拒、名单内的放行（ADR-0050）。这里的服务器经 Docker 网络别名互相直连，
/// 经核心转发组件的出站在 `proxy_tests.rs`。
#[test]
#[ignore = "需要 Docker，跑一遍约一分钟"]
fn the_real_server_hot_reloads_the_allowlist_from_the_second_file() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        // 服务器名的标签不能超过 DNS 的 63 字符上限（Tuwunel 启动就校验），取 24 个小写十六进制字符。
        let self_name = format!("{}.wonton.internal", &crate::secret::random_token()[..24]);
        let peer_name = format!("{}.wonton.internal", &crate::secret::random_token()[..24]);
        let stray_name = format!("{}.wonton.internal", &crate::secret::random_token()[..24]);
        let registration_secret = crate::secret::random_token();

        let dir = tempfile::tempdir().unwrap();
        // 本机名单来自第二份配置文件（fed/config.toml 的角色），另两台把本机放进各自名单。
        std::fs::write(
            dir.path().join("self.toml"),
            cluster_config(&self_name, &[], &registration_secret),
        )
        .unwrap();
        std::fs::write(
            dir.path().join("fed.toml"),
            allowlist_toml(&BTreeSet::from([peer_name.clone()])),
        )
        .unwrap();
        std::fs::write(
            dir.path().join("peer.toml"),
            cluster_config(&peer_name, &[&self_name], &registration_secret),
        )
        .unwrap();
        std::fs::write(
            dir.path().join("stray.toml"),
            cluster_config(&stray_name, &[&self_name], &registration_secret),
        )
        .unwrap();

        let cluster = Cluster::start(
            &dir,
            &[
                ("self.toml", Some("fed.toml")),
                ("peer.toml", None),
                ("stray.toml", None),
            ],
        )
        .await;
        let self_server = &cluster.servers[0];
        let fed_path = self_server.fed_path.as_ref().expect("本机才有 fed.toml");

        let self_user = register_user_on(&cluster, 0, "u0", &registration_secret).await;
        let peer_user = register_user_on(&cluster, 1, "v0", &registration_secret).await;

        // 初值名单里有对端：跨机邀请成立，对端能加入。
        let room = create_room(self_server, &self_user).await;
        invite(self_server, &self_user, &room, &peer_user.user_id)
            .await
            .unwrap();
        join(&cluster.servers[1], &peer_user.token, &room)
            .await
            .unwrap();

        // 名单外：本机名单直接拒（同步 403）。
        let stray_user = register_user_on(&cluster, 2, "w0", &registration_secret).await;
        let room = create_room(self_server, &self_user).await;
        let error = invite(self_server, &self_user, &room, &stray_user.user_id)
            .await
            .unwrap_err();
        assert!(error.contains("not allowed"), "{error}");

        // 重写第二份成空名单 + SIGUSR1：名单内的也被拒。
        std::fs::write(fed_path, allowlist_toml(&BTreeSet::new())).unwrap();
        cluster.signal_usr1(0).await;
        let fresh = register_user_on(&cluster, 1, "v1", &registration_secret).await;
        let room = create_room(self_server, &self_user).await;
        expect_invite_rejected(&cluster, &self_user, &room, &fresh.user_id).await;

        // 改回名单 + SIGUSR1：放行恢复，对端能加入。
        std::fs::write(
            fed_path,
            allowlist_toml(&BTreeSet::from([peer_name.clone()])),
        )
        .unwrap();
        cluster.signal_usr1(0).await;
        let fresh = register_user_on(&cluster, 1, "v2", &registration_secret).await;
        let room = create_room(self_server, &self_user).await;
        expect_invite_accepted(&cluster, &self_user, &room, &fresh).await;
    });
}

/// 一台测试服务器的最小联邦配置：TLS 自签证书和 dual_protocol 与正式安装一致（ADR-0050）。
/// 出站一律拨 TLS（实测：明文对端握手直接失败），没有 [global.tls] 就联邦不通。
fn cluster_config(server_name: &str, peers: &[&str], registration_secret: &str) -> String {
    let list = peers
        .iter()
        .map(|peer| format!("'^{}$'", regex_escape(peer)))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "[global]\nserver_name = \"{server_name}\"\n\
             database_path = \"/var/lib/tuwunel\"\naddress = \"0.0.0.0\"\nport = 8448\n\
             allow_federation = true\ntrusted_servers = []\nip_range_denylist = []\n\
             allow_registration = false\n\
             registration_shared_secret = \"{registration_secret}\"\n\
             allow_invalid_tls_certificates = true\n\
             allowed_remote_server_names_experimental = [{list}]\n\
             \n[global.tls]\n\
             certs = \"/etc/tuwunel/tls/tls.crt\"\n\
             key = \"/etc/tuwunel/tls/tls.key\"\n\
             # 测试的客户端 API 走明文（http://127.0.0.1:<映射端口>），和正式安装的 8008 一样。\n\
             dual_protocol = true\n"
    )
}

/// 从生成的配置里读服务器名（容器的网络别名要用它）。
fn config_server_name(dir: &tempfile::TempDir, file: &str) -> String {
    std::fs::read_to_string(dir.path().join(file))
        .unwrap()
        .lines()
        .find_map(|line| line.strip_prefix("server_name = \""))
        .and_then(|rest| rest.strip_suffix('"'))
        .expect("配置里有 server_name")
        .to_owned()
}

/// `docker run` 在镜像名之前的参数：网络别名、配置和证书挂载、代理置空。
fn docker_run_args(
    dir: &tempfile::TempDir,
    network: &str,
    index: usize,
    name: &str,
    server_name: &str,
    config_file: &str,
) -> Vec<String> {
    vec![
        "run".to_owned(),
        "--detach".to_owned(),
        "--name".to_owned(),
        name.to_owned(),
        "--publish".to_owned(),
        "127.0.0.1::8448".to_owned(),
        "--network".to_owned(),
        network.to_owned(),
        "--network-alias".to_owned(),
        server_name.to_owned(),
        "--volume".to_owned(),
        format!(
            "{}:/etc/tuwunel/tuwunel.toml:ro",
            dir.path().join(config_file).display()
        ),
        "--volume".to_owned(),
        format!(
            "{}:/etc/tuwunel/tls/tls.crt:ro",
            dir.path().join(format!("{index}.crt")).display()
        ),
        "--volume".to_owned(),
        format!(
            "{}:/etc/tuwunel/tls/tls.key:ro",
            dir.path().join(format!("{index}.key")).display()
        ),
        "--env".to_owned(),
        "TUWUNEL_CONFIG=/etc/tuwunel/tuwunel.toml".to_owned(),
        // 本机 Docker 客户端配了代理时会自动注入容器（大小写各一套），联邦出站会全走宿主
        // 代理然后失败（实测坑），全部置空。
        "--env".to_owned(),
        "HTTP_PROXY=".to_owned(),
        "--env".to_owned(),
        "HTTPS_PROXY=".to_owned(),
        "--env".to_owned(),
        "http_proxy=".to_owned(),
        "--env".to_owned(),
        "https_proxy=".to_owned(),
        "--env".to_owned(),
        "NO_PROXY=".to_owned(),
        "--env".to_owned(),
        "no_proxy=".to_owned(),
    ]
}

/// 在编号为 `index` 的测试服务器上注册一个用户。
async fn register_user_on(
    cluster: &Cluster,
    index: usize,
    user: &str,
    registration_secret: &str,
) -> Account {
    let server = &cluster.servers[index];
    register_user(
        &HttpClient::new(),
        &server.address,
        registration_secret,
        user,
    )
    .await
}

/// 热加载生效有先后：重试到邀请被本机名单拒（同步 403）为止。
async fn expect_invite_rejected(cluster: &Cluster, inviter: &Account, room: &str, user: &str) {
    let server = &cluster.servers[0];
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    while std::time::Instant::now() < deadline {
        match invite(server, inviter, room, user).await {
            Ok(()) => tokio::time::sleep(std::time::Duration::from_millis(500)).await,
            Err(error) => {
                assert!(error.contains("not allowed"), "{error}");
                return;
            }
        }
    }
    panic!("热加载后名单内的邀请没有被拒");
}

/// 重试到邀请放行且对端加入成功为止。
async fn expect_invite_accepted(cluster: &Cluster, inviter: &Account, room: &str, guest: &Account) {
    let server = &cluster.servers[0];
    let peer = &cluster.servers[1];
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    while std::time::Instant::now() < deadline {
        if invite(server, inviter, room, &guest.user_id).await.is_ok()
            && join(peer, &guest.token, room).await.is_ok()
        {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
    panic!("恢复名单后对端没有加入");
}

/// 一台测试用的 Tuwunel：容器名、映射出来的地址、挂进去的名单文件。
pub(super) struct ClusterServer {
    pub(super) name: String,
    pub(super) address: String,
    pub(super) fed_path: Option<std::path::PathBuf>,
}

/// 一组测试容器和它们的专用网络，Drop 时全部删掉。
struct Cluster {
    network: String,
    servers: Vec<ClusterServer>,
}

impl Cluster {
    /// `files` 按顺序起容器；`Some(fed 名字)` 的那台按两份配置启动（-c 两次）。
    /// 先持有 `Cluster` 再逐台加：中途失败（包括 panic）Drop 会删掉已经起的容器。
    async fn start(dir: &tempfile::TempDir, files: &[(&str, Option<&str>)]) -> Self {
        let network = format!("fedcfg-test-{}", crate::secret::random_token());
        let created = Command::new("docker")
            .args(["network", "create", &network])
            .output()
            .expect("运行 Docker");
        assert!(
            created.status.success(),
            "建测试网络：{}",
            String::from_utf8_lossy(&created.stderr)
        );
        let mut cluster = Self {
            network,
            servers: Vec::new(),
        };
        for (index, (file, fed)) in files.iter().enumerate() {
            let server = Self::start_server(dir, &cluster.network, index, file, *fed).await;
            cluster.servers.push(server);
        }
        cluster
    }
    /// 起一台测试服务器：生成自签证书、按两份配置（可选）挂载、映射端口、等就绪。
    async fn start_server(
        dir: &tempfile::TempDir,
        network: &str,
        index: usize,
        file: &str,
        fed: Option<&str>,
    ) -> ClusterServer {
        let name = format!("fedcfg-test-{index}-{}", crate::secret::random_token());
        // 别名就是服务器名：没有它，embedded DNS 解析不了对端名（hosts 只挂在本机那台）。
        let server_name = config_server_name(dir, file);
        // 配置里 [global.tls] 指向的证书：和正式安装一样是自签的（ADR-0050）。
        let tls =
            rcgen::generate_simple_self_signed(vec![server_name.clone()]).expect("生成自签证书");
        std::fs::write(dir.path().join(format!("{index}.crt")), tls.cert.pem()).unwrap();
        std::fs::write(
            dir.path().join(format!("{index}.key")),
            tls.signing_key.serialize_pem(),
        )
        .unwrap();
        let mut docker_args = docker_run_args(dir, network, index, &name, &server_name, file);
        // 镜像名后面的都是 tuwunel 自己的命令行。
        let mut command_args = vec!["-c".to_owned(), "/etc/tuwunel/tuwunel.toml".to_owned()];
        let fed_path = match fed {
            Some(fed) => {
                let path = dir.path().join(fed);
                let mount = format!("{}:/etc/tuwunel/fed.toml:ro", path.display());
                docker_args.push("--volume".to_owned());
                docker_args.push(mount);
                command_args.push("-c".to_owned());
                command_args.push("/etc/tuwunel/fed.toml".to_owned());
                Some(path)
            }
            None => None,
        };
        docker_args.push(image());
        docker_args.extend(command_args);
        let output = Command::new("docker")
            .args(&docker_args)
            .output()
            .expect("运行 Docker");
        assert!(
            output.status.success(),
            "启动 Tuwunel：{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let mapped = Command::new("docker")
            .args(["port", &name, "8448/tcp"])
            .output()
            .unwrap();
        let logs = Command::new("docker")
            .args(["logs", "--tail", "5", &name])
            .output()
            .map(|logs| {
                format!(
                    "{}{}",
                    String::from_utf8_lossy(&logs.stdout),
                    String::from_utf8_lossy(&logs.stderr)
                )
            })
            .unwrap_or_default();
        assert!(mapped.status.success(), "读取 Tuwunel 端口失败：{logs}");
        let address = format!(
            "http://{}",
            String::from_utf8(mapped.stdout).unwrap().trim()
        );
        let server = ClusterServer {
            name,
            address,
            fed_path,
        };
        wait_ready(&server.address).await;
        server
    }

    async fn signal_usr1(&self, index: usize) {
        let engine = Engine::connect().expect("连接 Docker");
        engine
            .kill_container(&self.servers[index].name, "SIGUSR1")
            .await
            .expect("发 SIGUSR1");
    }
}

impl Drop for Cluster {
    fn drop(&mut self) {
        for server in &self.servers {
            let _ = Command::new("docker")
                .args(["rm", "--force", "--volumes", &server.name])
                .output();
        }
        let _ = Command::new("docker")
            .args(["network", "rm", &self.network])
            .output();
    }
}

pub(super) async fn wait_ready(base: &str) {
    let http = HttpClient::new();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    while std::time::Instant::now() < deadline {
        if let Ok((StatusCode::OK, _)) = http
            .json(
                Method::GET,
                &format!("{base}/_matrix/client/versions"),
                None,
                WAIT,
            )
            .await
        {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
    panic!("Tuwunel 没有就绪：{base}");
}

/// 共享密钥注册一个普通用户，返回访问令牌和用户 ID（Synapse 兼容的管理接口，ADR-0009）。
pub(super) async fn register_user(
    http: &HttpClient,
    base: &str,
    shared_secret: &str,
    user: &str,
) -> Account {
    use aws_lc_rs::hmac;
    let (status, value) = http
        .json(
            Method::GET,
            &format!("{base}/_synapse/admin/v1/register"),
            None,
            WAIT,
        )
        .await
        .unwrap();
    assert_eq!(status, StatusCode::OK, "{value}");
    let nonce = value["nonce"].as_str().expect("有 nonce").to_owned();
    let password = crate::secret::random_token();
    // 普通账号的签名字段以 notadmin 结尾，管理账号才是 admin（admin.rs）。
    let message = [nonce.as_str(), user, password.as_str(), "notadmin"].join("\0");
    let key = hmac::Key::new(
        hmac::HMAC_SHA1_FOR_LEGACY_USE_ONLY,
        shared_secret.as_bytes(),
    );
    let mac: String = hmac::sign(&key, message.as_bytes())
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    let (status, value) = http
        .json(
            Method::POST,
            &format!("{base}/_synapse/admin/v1/register"),
            Some(&serde_json::json!({
                "nonce": nonce,
                "username": user,
                "password": password,
                "mac": mac,
            })),
            WAIT,
        )
        .await
        .unwrap();
    assert_eq!(status, StatusCode::OK, "{value}");
    Account {
        token: value["access_token"].as_str().expect("有令牌").to_owned(),
        user_id: value["user_id"].as_str().expect("有用户 ID").to_owned(),
    }
}

pub(super) struct Account {
    pub(super) token: String,
    pub(super) user_id: String,
}

pub(super) async fn create_room(server: &ClusterServer, account: &Account) -> String {
    let http = HttpClient::new();
    let (_, value) = http
        .request_json(
            Method::POST,
            &format!("{}/_matrix/client/v3/createRoom", server.address),
            Some(&account.token),
            RequestBody::Json(&serde_json::json!({"preset": "private_chat"})),
            WAIT,
        )
        .await
        .unwrap();
    value["room_id"].as_str().expect("有房间 ID").to_owned()
}

/// 邀请；被本机名单拒绝时返回带原因的错误（同步 403）。
pub(super) async fn invite(
    server: &ClusterServer,
    account: &Account,
    room: &str,
    user_id: &str,
) -> Result<(), String> {
    let http = HttpClient::new();
    let (status, value) = http
        .request_json(
            Method::POST,
            &format!("{}/_matrix/client/v3/rooms/{room}/invite", server.address),
            Some(&account.token),
            RequestBody::Json(&serde_json::json!({"user_id": user_id})),
            WAIT,
        )
        .await
        .map_err(|error| format!("请求失败：{error}"))?;
    if status == StatusCode::OK {
        return Ok(());
    }
    Err(format!(
        "邀请返回 {status}：{}",
        value["error"].as_str().unwrap_or_default()
    ))
}

/// 对端用户接受邀请并加入：跨机握手走通的唯一客户端可见证据。
pub(super) async fn join(server: &ClusterServer, token: &str, room: &str) -> Result<(), String> {
    let http = HttpClient::new();
    // 同步拿到邀请，再加入。
    let (_, value) = http
        .request_json(
            Method::GET,
            &format!("{}/_matrix/client/v3/sync", server.address),
            Some(token),
            RequestBody::Empty,
            WAIT,
        )
        .await
        .map_err(|error| format!("同步失败：{error}"))?;
    let invited = value["rooms"]["invite"]
        .as_object()
        .map(|rooms| rooms.keys().any(|id| id == room))
        .unwrap_or(false);
    if !invited {
        return Err(format!("没等到房间 {room} 的邀请"));
    }
    let (status, value) = http
        .request_json(
            Method::POST,
            &format!("{}/_matrix/client/v3/rooms/{room}/join", server.address),
            Some(token),
            RequestBody::Json(&serde_json::json!({})),
            WAIT,
        )
        .await
        .map_err(|error| format!("加入失败：{error}"))?;
    if status == StatusCode::OK {
        return Ok(());
    }
    Err(format!(
        "加入返回 {status}：{}",
        value["error"].as_str().unwrap_or_default()
    ))
}
