//! 测试用的 Tuwunel、模拟核心的签发与管理，以及等待异步结果的工具。
//!
//! 只在 `testkit` 特性下编译，给本 crate 和客户端核心的集成测试共用；App 不启用。

mod admin;
mod issuer;
mod keys;
mod push;
mod relay;
mod to_device;

use std::{fmt::Debug, path::PathBuf, time::Duration};

use crate::{MatrixSession, Result, SessionConfig, StoreConfig};
use futures_util::{Stream, StreamExt};
use testcontainers::{
    ContainerAsync, GenericImage, ImageExt, TestcontainersError,
    core::{Host, IntoContainerPort, WaitFor},
    runners::AsyncRunner,
};

pub use self::{
    admin::{Admin, diagnose_live_session},
    issuer::{Claims, Issuer, now},
    push::{PushGateway, Received},
    relay::Relay,
    to_device::ToDeviceHold,
};

/// 本会话当前生效的令牌：（访问令牌，刷新令牌）。刷新令牌登录时申请过才有；停用后旧会话
/// 还能建房间时，诊断用这对旧令牌裸打 whoami/refresh，看服务器当时认不认。
impl MatrixSession {
    pub fn tokens(&self) -> Option<(String, Option<String>)> {
        self.client
            .session_tokens()
            .map(|tokens| (tokens.access_token, tokens.refresh_token))
    }

    /// 建一个不加密的群聊（`private_chat` 预设）并邀请 `peer`：和私聊不同，被邀请的人加入后是普通成员（power level 0），
    /// 撤回不了别人的消息。
    pub async fn create_group_room(
        &self,
        name: &str,
        peer: &matrix_sdk::ruma::UserId,
    ) -> Result<matrix_sdk::ruma::OwnedRoomId> {
        use matrix_sdk::ruma::api::client::room::create_room::v3::{Request, RoomPreset};
        let mut request = Request::new();
        request.name = Some(name.to_owned());
        request.invite = vec![peer.to_owned()];
        request.preset = Some(RoomPreset::PrivateChat);
        let room = self.client.create_room(request).await?;
        Ok(room.room_id().to_owned())
    }
}

/// 本地开发环境的配置和镜像，测试用同一份。
const COMPOSE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../deploy/dev/matrix/compose.yaml"
));
const CONFIG: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../deploy/dev/matrix/tuwunel.toml"
));
const CONFIG_PATH: &str = "/etc/tuwunel/tuwunel.toml";
const PORT: u16 = 8008;

/// `WONTON_TEST_LOG` 有值时打开日志（值是 tracing 的过滤规则），写到标准错误；失败的时候按时间顺序留下当时发生了什么。
fn init_test_logging() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let filter = std::env::var("WONTON_TEST_LOG").unwrap_or_else(|_| "info".to_owned());
        let filter = tracing_subscriber::EnvFilter::try_new(filter)
            .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn"));
        tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_ansi(false)
            .with_writer(std::io::stderr)
            .try_init()
            .ok();
    });
}

/// 等待异步结果的上限。正常情况下一两秒内就有结果；吊销设备后同步要等进行中的长轮询返回。
const WAIT: Duration = Duration::from_secs(60);

/// 一台供测试使用的 Tuwunel，以及持有它签发密钥的“核心”。
pub struct TestServer {
    pub url: String,
    pub issuer: Issuer,
    registration_secret: Option<String>,
    _container: Option<ContainerAsync<GenericImage>>,
}

impl TestServer {
    /// 默认用 Docker 起一台一次性的 Tuwunel。
    ///
    /// 设置了 `WONTON_MATRIX_HOMESERVER` 时改连现成的服务器（例如 dev.sh 起的服务器，或经隧道的
    /// 本地代理），用 `WONTON_MATRIX_JWT_KEY` 指向的私钥签发，默认是 dev.sh 生成的密钥。
    pub async fn start() -> Self {
        init_test_logging();
        let Ok(url) = std::env::var("WONTON_MATRIX_HOMESERVER") else {
            return Self::container(&[]).await;
        };
        let key_path = std::env::var("WONTON_MATRIX_JWT_KEY")
            .map(PathBuf::from)
            .unwrap_or_else(|_| {
                PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join("../../deploy/dev/matrix/.secrets/jwt-ed25519.pem")
            });
        let pem = std::fs::read(&key_path)
            .unwrap_or_else(|error| panic!("读取签发私钥 {}：{error}", key_path.display()));
        Self {
            url,
            issuer: Issuer::from_private_key_pem(&pem),
            registration_secret: None,
            _container: None,
        }
    }

    /// 用 Docker 起一台一次性的 Tuwunel，`overrides` 以环境变量形式覆盖配置。
    pub async fn container(overrides: &[(&str, &str)]) -> Self {
        Self::start_container(overrides, false)
            .await
            .unwrap_or_else(|error| start_failed(&error))
    }

    /// 能把推送发到 `gateway`（测试进程里扮演推送代理的推送网关）的 Tuwunel：经只代理推送代理域名的出站代理，
    /// 和产品里一样。
    ///
    /// 容器要连回测试进程。Linux 上宿主的防火墙（比如 ufw）常常挡住容器到宿主的连接，回环地址不受影响，所以这台
    /// Tuwunel 用宿主的网络、只听回环地址；Docker Desktop、OrbStack 上容器经 `host.docker.internal` 到宿主。
    pub async fn with_push_gateway(gateway: &PushGateway) -> Self {
        let proxy = gateway.tuwunel_proxy();
        if !cfg!(target_os = "linux") {
            return Self::container(&[("TUWUNEL_PROXY", proxy.as_str())]).await;
        }
        // 端口交给 Tuwunel 自己挑（`TUWUNEL_PORT=0`）：先挑再交出去，到它真绑之间隔着几百毫秒，宿主上别的进程
        // （出站连接也从这段临时端口里取）能把它抢走，Tuwunel 就绑不上。
        let overrides = [
            ("TUWUNEL_PROXY", proxy.as_str()),
            ("TUWUNEL_ADDRESS", "127.0.0.1"),
            ("TUWUNEL_PORT", "0"),
        ];
        Self::start_container(&overrides, true)
            .await
            .unwrap_or_else(|error| start_failed(&error))
    }

    /// `host_network` 为真时容器用宿主的网络，端口由 Tuwunel 自己挑、从它的日志里读；否则容器在自己的网络里，
    /// 听 8008，宿主经容器的地址连（Docker Desktop 这类到不了容器地址的平台才发布到宿主端口）。
    async fn start_container(
        overrides: &[(&str, &str)],
        host_network: bool,
    ) -> std::result::Result<Self, TestcontainersError> {
        let issuer = Issuer::generate();
        let secret = random_hex(32);
        let (name, tag) = image();
        let image = GenericImage::new(name, tag)
            .with_wait_for(WaitFor::message_on_either_std("Listening on"));
        // 走 Docker 的网络时，只在宿主到不了容器地址的平台（Docker Desktop、OrbStack）才发布到宿主端口。
        let image = match host_network {
            true => image,
            false if cfg!(target_os = "linux") => image,
            false => image.with_exposed_port(PORT.tcp()),
        };
        let mut request = image
            .with_copy_to(CONFIG_PATH, CONFIG.as_bytes().to_vec())
            .with_env_var("TUWUNEL_CONFIG", CONFIG_PATH)
            .with_env_var("TUWUNEL_JWT__KEY", issuer.public_key_pem())
            .with_env_var("TUWUNEL_REGISTRATION_SHARED_SECRET", &secret)
            // 这个标签登记是哪个测试进程起的：testcontainers 起的名字是随机的，本机多套测试并行时
            // 认不出是谁的容器。
            .with_label("wonton.test-pid", std::process::id().to_string());
        request = match host_network {
            true => request.with_network("host"),
            // Docker Desktop、OrbStack 自带这个名字，Linux 上要显式加。
            false => request.with_host("host.docker.internal", Host::HostGateway),
        };
        for (key, value) in overrides {
            request = request.with_env_var(*key, *value);
        }
        let container = request.start().await?;
        let url = match host_network {
            true => format!("http://127.0.0.1:{}", listening_port(&container).await),
            // 测试进程在 Docker 的宿主上，直接走容器的地址：不发布宿主端口，就没有 Docker 挑中的那个号被宿主上
            // 别的套接字占掉这回事——临时端口段里一直在发生，配上就是 `address already in use`。
            false if cfg!(target_os = "linux") => {
                let ip = container.get_bridge_ip_address().await.expect("容器地址");
                format!("http://{ip}:{PORT}")
            }
            false => {
                let host = container.get_host().await.expect("容器地址");
                let port = container.get_host_port_ipv4(PORT).await.expect("容器端口");
                format!("http://{host}:{port}")
            }
        };
        Ok(Self {
            url,
            issuer,
            registration_secret: Some(secret),
            _container: Some(container),
        })
    }

    pub fn config(&self, device_name: &str, store: StoreConfig) -> SessionConfig {
        SessionConfig {
            homeserver_url: self.url.clone(),
            device_name: device_name.to_owned(),
            store,
            proxy: None,
        }
    }

    /// 客户端用核心签发的 JWT 登录。
    pub async fn try_login(&self, jwt: &str) -> Result<MatrixSession> {
        MatrixSession::login_with_jwt(&self.config("测试设备", StoreConfig::InMemory), jwt).await
    }

    /// 核心为 `localpart` 签发登录令牌，客户端用它登录一台新设备。
    pub async fn login(&self, localpart: &str, device_name: &str) -> MatrixSession {
        self.login_with_store(localpart, device_name, StoreConfig::InMemory)
            .await
    }

    pub async fn login_with_store(
        &self,
        localpart: &str,
        device_name: &str,
        store: StoreConfig,
    ) -> MatrixSession {
        self.login_via(&self.url, localpart, device_name, store)
            .await
    }

    /// 经 `homeserver_url`（比如 [`Relay`] 的地址）登录，之后的请求都走它。
    pub async fn login_via(
        &self,
        homeserver_url: &str,
        localpart: &str,
        device_name: &str,
        store: StoreConfig,
    ) -> MatrixSession {
        let config = SessionConfig {
            homeserver_url: homeserver_url.to_owned(),
            ..self.config(device_name, store)
        };
        let jwt = self.issuer.login_token(localpart);
        MatrixSession::login_with_jwt(&config, &jwt)
            .await
            .unwrap_or_else(|error| panic!("{localpart} 登录失败：{error}"))
    }

    /// 冻住服务器：连接还能建立，但请求永远等不到回应，相当于隧道断开时请求挂住。只有测试自己起的容器能冻。
    pub async fn freeze(&self) {
        self._container
            .as_ref()
            .expect("现成的服务器不能冻住")
            .pause()
            .await
            .expect("冻住 Tuwunel 容器");
    }

    /// 注册一个用密码登录的账号（用现成 Matrix 账号登录的测试用），返回完整用户 ID 和密码。容器要开着密码登录
    /// （`TUWUNEL_LOGIN_WITH_PASSWORD=true`），只有测试自己起的容器能注册。
    pub async fn register_password_user(&self, localpart: &str) -> (String, String) {
        let secret = self
            .registration_secret
            .as_deref()
            .expect("现成的服务器没有注册密钥");
        let password = random_hex(16);
        let response = admin::register_user(
            &matrix_sdk::reqwest::Client::new(),
            &self.url,
            secret,
            localpart,
            &password,
            false,
        )
        .await;
        let user_id = response["user_id"]
            .as_str()
            .expect("注册接口应返回用户 ID")
            .to_owned();
        (user_id, password)
    }

    /// 核心的管理账号，只有测试自己起的容器才有。
    pub async fn admin(&self) -> Admin {
        let secret = self
            .registration_secret
            .as_deref()
            .expect("现成的服务器没有管理密钥");
        Admin::register(&self.url, secret).await
    }
}

/// 起不来时把 Docker 的原始报错连同本机需要的东西一起抛出去。
fn start_failed(error: &TestcontainersError) -> ! {
    panic!(
        "启动 Tuwunel 容器失败：{error}。测试需要 Docker；testcontainers 不读 docker context，\
         套接字不在 /var/run/docker.sock 时先运行 \
         export DOCKER_HOST=\"$(docker context inspect --format '{{{{.Endpoints.docker.Host}}}}')\""
    )
}

/// Tuwunel 挑上的端口：它把 `Listening on ["tcp:127.0.0.1:端口"]` 打在 stdout 上，绑好了才打。
async fn listening_port(container: &ContainerAsync<GenericImage>) -> u16 {
    let stdout = container
        .stdout_to_vec()
        .await
        .expect("读 Tuwunel 的 stdout");
    let text = String::from_utf8_lossy(&stdout);
    text.lines()
        .rev()
        .find_map(|line| {
            let (_, port) = line.split_once("Listening on [\"tcp:127.0.0.1:")?;
            port.split('"').next()?.parse().ok()
        })
        .unwrap_or_else(|| panic!("Tuwunel 的日志里没有它挑上的端口：{text}"))
}

/// compose.yaml 里的镜像，拆成名字和标签；CI 经 `WONTON_TEST_IMAGES` 指到镜像仓库副本
/// （wonton-test-images），本地按 compose 里的上游引用拉。摘要先去掉再拆：testcontainers 只收标签。
fn image() -> (String, String) {
    let (name, tag) = COMPOSE
        .lines()
        .find_map(|line| line.trim().strip_prefix("image: "))
        .and_then(|image| {
            image
                .split_once('@')
                .map_or(image, |(r, _)| r)
                .rsplit_once(':')
        })
        .expect("compose.yaml 里应有带标签的 Tuwunel 镜像");
    let mapped = wonton_test_images::image(&format!("{name}:{tag}"));
    mapped
        .rsplit_once(':')
        .map(|(name, tag)| (name.to_owned(), tag.to_owned()))
        .expect("镜像引用应有标签")
}

/// 每次运行都不同的用户名，测试之间、多次运行之间互不干扰。
pub fn unique(prefix: &str) -> String {
    format!("{prefix}-{}", random_hex(4))
}

pub fn random_bytes(len: usize) -> Vec<u8> {
    let mut bytes = vec![0; len];
    aws_lc_rs::rand::fill(&mut bytes).expect("生成随机数");
    bytes
}

pub fn random_hex(len: usize) -> String {
    random_bytes(len)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// 测试结束时删除的 SQLite 存储目录。口令在创建时定下，重启后用同一个口令打开。
pub struct TempStore {
    path: PathBuf,
    passphrase: String,
}

impl TempStore {
    pub fn create() -> Self {
        Self {
            path: std::env::temp_dir().join(unique("wonton-matrix-store")),
            passphrase: random_hex(16),
        }
    }

    pub fn config(&self) -> StoreConfig {
        StoreConfig::Sqlite {
            path: self.path.clone(),
            passphrase: Some(self.passphrase.clone()),
        }
    }
}

impl Drop for TempStore {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// 等流里出现满足条件的一项，返回 `check` 取出的结果；超时就失败并打印最后一项。
pub async fn eventually<T: Debug, R>(
    what: &str,
    stream: impl Stream<Item = T>,
    mut check: impl FnMut(&T) -> Option<R>,
) -> R {
    let mut last = None;
    let waiting = async {
        let mut stream = std::pin::pin!(stream);
        while let Some(item) = stream.next().await {
            if let Some(found) = check(&item) {
                return Some(found);
            }
            last = Some(item);
        }
        None
    };
    match tokio::time::timeout(WAIT, waiting).await {
        Ok(Some(found)) => found,
        Ok(None) => panic!("{what}：流已结束，最后一项：{last:#?}"),
        Err(_) => panic!("{what}：{WAIT:?} 内没有等到，最后一项：{last:#?}"),
    }
}

/// 造一条测试用的图片消息内容（未加密来源）。上游测试（如桥的行为测试）要往消息流里
/// 放图片时用它；生产代码走不了这条路（`source` 字段不公开）。
impl crate::ImageContent {
    pub fn for_test(
        filename: &str,
        mime_type: Option<&str>,
        size: Option<u64>,
        caption: Option<&str>,
        mxc: &str,
    ) -> Self {
        Self {
            filename: filename.to_owned(),
            mime_type: mime_type.map(str::to_owned),
            size,
            width: None,
            height: None,
            caption: caption.map(str::to_owned),
            source: plain_source(mxc),
            thumbnail: None,
        }
    }
}

/// 造一条测试用的文件消息内容（未加密来源）；用途同 [`crate::ImageContent::for_test`]。
impl crate::FileContent {
    pub fn for_test(
        filename: &str,
        mime_type: Option<&str>,
        size: Option<u64>,
        caption: Option<&str>,
        mxc: &str,
    ) -> Self {
        Self {
            filename: filename.to_owned(),
            mime_type: mime_type.map(str::to_owned),
            size,
            caption: caption.map(str::to_owned),
            source: plain_source(mxc),
        }
    }
}

fn plain_source(mxc: &str) -> matrix_sdk::ruma::events::room::MediaSource {
    matrix_sdk::ruma::events::room::MediaSource::Plain(mxc.to_owned().into())
}
