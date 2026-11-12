//! 入口容器和连接器：核心按期望状态经 Docker API 建和删（ADR-0021 的“组件”）。
//!
//! 不写在主 compose 里：开关、换连接器、地址变化都要在运行时重建，compose 项目 `up` 过一次就不再变（ADR-0021
//! “入口容器、局域网转发、连接器都由核心管”）。网络仍由主 compose 渲染，核心只把容器接上去；这样 `WONTON_EXCLUDE_LISTEN`
//! 和 8484 的绑定在启动时就能定。
//!
//! 核心不在容器里（开发时没设 `WONTON_CORE_CONTAINER`）时没有 compose 给的那些网络，要入口就回
//! `core_not_in_container`，不降级、不绑 `0.0.0.0`。

use std::collections::HashMap;
use std::fmt;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

use bollard::models::{
    ContainerCreateBody, EndpointIpamConfig, EndpointSettings, HostConfig, HostConfigLogConfig,
    NetworkCreateRequest, NetworkingConfig, PortBinding, RestartPolicy, RestartPolicyNameEnum,
};

use crate::entry_haproxy::{CONNECTOR_ADDRESS, ENTRY_EDGE_ADDRESS};
use crate::entry_settings::EntrySettings;
use crate::plugin::compose::PullPolicy;
use crate::plugin::core_address::Endpoint;
use crate::plugin::docker::{Engine, EngineError};
use crate::plugin::names::{Instance, LABEL_COMPONENT, LABEL_MANAGED_BY};
use bollard::models::ContainerInspectResponse;

/// 组件的标签值，也是镜像 `wonton.core.components` 里声明的名字。
pub const ENTRY_COMPONENT: &str = "entry";
pub const CONNECTOR_COMPONENT: &str = "connector";

pub const ENTRY_EDGE_ALIAS: &str = "entry-edge.wonton.internal";
pub const ENTRY_CLIENTS_ALIAS: &str = "entry.wonton.internal";
pub const CONNECTOR_ALIAS: &str = "connector.wonton.internal";

/// 连接器把公网 TCP 转发接进入口的端口（路由器和自建 frps 都用）。
const CONNECTOR_PORT: u32 = 8443;

#[derive(Debug)]
pub enum EntryError {
    /// 核心跑在宿主上：没有 compose 的 `clients`/`edge`，做不了入口。
    CoreNotInContainer,
    /// 同名容器是别的核心或主人的，标签不是我们的。
    NameTaken(String),
    /// compose 该渲染的网络不在。
    NetworkMissing(String),
    /// 入口容器不唯一，热加载不知道该发给谁。
    NotOneEntry(usize),
    Docker(EngineError),
}

impl EntryError {
    /// ADR-0017 的错误类别（C2 定了 `core_not_in_container` 等几条）。
    pub fn category(&self) -> &'static str {
        match self {
            Self::CoreNotInContainer | Self::NetworkMissing(_) => "core_not_in_container",
            Self::NameTaken(_) | Self::NotOneEntry(_) => "conflict",
            Self::Docker(EngineError::PortTaken(_)) => "conflict",
            Self::Docker(EngineError::Rejected { .. }) => "invalid_input",
            Self::Docker(EngineError::NotFound(_)) => "not_found",
            Self::Docker(EngineError::Pull { .. }) => "source_unavailable",
            Self::Docker(EngineError::Unavailable(_) | EngineError::TimedOut(_)) => {
                "docker_unavailable"
            }
            Self::Docker(EngineError::Unsupported(_)) => "invalid_input",
        }
    }
}

impl fmt::Display for EntryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CoreNotInContainer => write!(
                f,
                "{}：核心不在容器里，没有入口要的 clients、edge 网络",
                self.category()
            ),
            Self::NameTaken(name) => write!(f, "{}：{name} 已被别的资源占用", self.category()),
            Self::NetworkMissing(network) => {
                write!(
                    f,
                    "{}：compose 该渲染的网络 {network} 不在",
                    self.category()
                )
            }
            Self::NotOneEntry(count) => write!(
                f,
                "{}：找到 {count} 个入口容器，热加载不知道发给谁",
                self.category()
            ),
            Self::Docker(error) => write!(f, "{}：{error}", self.category()),
        }
    }
}

impl std::error::Error for EntryError {}

impl From<EngineError> for EntryError {
    fn from(error: EngineError) -> Self {
        Self::Docker(error)
    }
}

/// 一侧容器的期望状态：接哪些网络、用什么镜像。
struct Side {
    component: &'static str,
    image: &'static str,
    body: ContainerCreateBody,
    /// 接进的网络、固定地址、别名和网关优先级。
    endpoints: Vec<(String, Endpoint)>,
}

pub struct EntryContainers {
    engine: Engine,
    install_dir: PathBuf,
    data_dir: PathBuf,
    instance: Instance,
    core_container: Option<String>,
    /// 连接器发布的地址和端口；空时看 `WONTON_ENTRY_BIND`/`WONTON_ENTRY_PORT`。
    binding: Option<(String, String)>,
}

impl EntryContainers {
    pub fn new(
        engine: Engine,
        install_dir: PathBuf,
        data_dir: PathBuf,
        instance: Instance,
        core_container: Option<String>,
    ) -> Self {
        Self {
            engine,
            install_dir,
            data_dir,
            instance,
            core_container,
            binding: None,
        }
    }

    /// 测试用：固定发布地址和端口，不读进程环境。
    pub fn with_binding(mut self, bind: &str, port: &str) -> Self {
        self.binding = Some((bind.to_owned(), port.to_owned()));
        self
    }

    pub fn engine(&self) -> Engine {
        self.engine.clone()
    }

    pub fn install_dir(&self) -> &Path {
        &self.install_dir
    }

    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    /// 有联邦或者任何一个打开的入口就要入口容器；有连接器设置就要连接器。
    ///
    /// `settings` 只在有名字时装机写下，那时 `is_private` 一定是假，所以这里目前恒开；入口的开关（ADR-0021）要等设置里
    /// 有开关以后才可能关掉它。连接器同样：家名服务（frpc）和路由器转发（发布端口的 HAProxy）现在都随设置存在，
    /// ADR-0090 的总开关接上后再由设置决定。
    fn wanted(settings: &EntrySettings) -> (bool, bool) {
        let federation = !crate::matrix::federation_config::is_private(&settings.domain);
        (federation, true)
    }

    /// 按设置对账：缺的建、多的删、镜像换了重建、网络没接上就接上。文件由 `wonton-core init`（或改了开关的
    /// 操作）先写好，这里只读。
    pub async fn reconcile(&self, settings: Option<&EntrySettings>) -> Result<(), EntryError> {
        let (entry, connector) = match settings {
            Some(settings) => Self::wanted(settings),
            None => (false, false),
        };
        let Some(core) = &self.core_container else {
            // 没有设置、也没有容器网络：开发时跑在宿主上，什么都不用做。
            return if entry || connector {
                Err(EntryError::CoreNotInContainer)
            } else {
                Ok(())
            };
        };
        let mut resolved = None;
        let sides: [(bool, &'static str); 2] =
            [(entry, ENTRY_COMPONENT), (connector, CONNECTOR_COMPONENT)];
        for (wanted, component) in sides {
            let name = self.name(component);
            if !wanted {
                self.engine.remove_container(&name).await?;
                continue;
            }
            if resolved.is_none() {
                resolved = Some(self.networks(core).await?);
            }
            let (project, networks) = resolved.as_ref().expect("网络刚取过");
            let settings = settings.expect("要有设置才可能有容器");
            if networks.connector != CONNECTOR_ADDRESS {
                self.rewrite_config(settings, &networks.connector)?;
            }
            let side = self.side(component, networks, settings)?;
            self.ensure(&side, project).await?;
        }
        Ok(())
    }

    /// 拆栈和卸载时按组件标签删掉两个容器：它们挂着网络，compose `down` 删不掉。
    pub async fn remove_all(&self) -> Result<(), EntryError> {
        for component in [ENTRY_COMPONENT, CONNECTOR_COMPONENT] {
            let found = self
                .engine
                .labeled(LABEL_MANAGED_BY, "core", |labels| {
                    self.instance.owns(labels)
                        && labels.get(LABEL_COMPONENT).map(String::as_str) == Some(component)
                })
                .await?;
            for name in found.containers {
                self.engine.remove_container(&name).await?;
            }
            for network in found.networks {
                self.engine.remove_network(&network).await?;
            }
        }
        Ok(())
    }

    /// 证书续期后让 HAProxy 热加载配置（`USR2`：已升级的连接不断，实测）。
    pub async fn reload(&self) -> Result<(), EntryError> {
        let found = self
            .engine
            .labeled(LABEL_MANAGED_BY, "core", |labels| {
                self.instance.owns(labels)
                    && labels.get(LABEL_COMPONENT).map(String::as_str) == Some(ENTRY_COMPONENT)
            })
            .await?;
        match found.containers.as_slice() {
            [name] => {
                self.engine.kill_container(name, "SIGUSR2").await?;
                Ok(())
            }
            other => Err(EntryError::NotOneEntry(other.len())),
        }
    }

    /// 容器的名字：和插件资源一样带实例名，几套核心共用一台 Docker 时不会撞。
    fn name(&self, component: &str) -> String {
        match self.instance.name() {
            None => format!("wonton-{component}"),
            Some(instance) => format!("wonton-{instance}--{component}"),
        }
    }

    /// compose 给主项目渲染的网络名：`<项目>_clients`。项目名从核心自己的标签读，开发栈和正式安装不写死；
    /// 项目名也用来认上一版 compose 建的同名容器。
    async fn networks(&self, core: &str) -> Result<(String, Networks), EntryError> {
        let labels = self
            .engine
            .inspect_container(core)
            .await?
            .and_then(|info| info.config)
            .and_then(|config| config.labels)
            .unwrap_or_default();
        let project = labels
            .get("com.docker.compose.project")
            .ok_or_else(|| EntryError::NetworkMissing("compose 项目标签".to_owned()))?
            .clone();
        let network = |key: &str| format!("{project}_{key}");
        let mut networks = Networks {
            clients: network("clients"),
            edge: network("edge"),
            relay: network("relay"),
            connector: CONNECTOR_ADDRESS.to_owned(),
            entry: ENTRY_EDGE_ADDRESS.to_owned(),
        };
        // `clients` 核心自己在用，compose 会建；`edge`、`relay` 没有任何 compose 服务引用，compose 跳过它们
        // （实测：全新安装时这两张网络不存在），所以由核心按原来的参数建。
        if self
            .engine
            .inspect_network(&networks.clients)
            .await?
            .is_none()
        {
            return Err(EntryError::NetworkMissing(networks.clients.clone()));
        }
        let subnet = self
            .ensure_network(
                &networks.edge,
                Some(crate::entry_haproxy::EDGE_SUBNET),
                true,
            )
            .await?
            .unwrap_or_else(|| crate::entry_haproxy::EDGE_SUBNET.to_owned());
        // 固定地址跟着实际网段走：同一台机器上另起一套云吞时冒烟会把网段改掉（`edge` 的默认网段只能有一份）。
        let base = subnet
            .split('/')
            .next()
            .and_then(|address| address.parse::<std::net::Ipv4Addr>().ok())
            .unwrap_or_else(|| "10.90.0.0".parse().expect("固定字符串"));
        let offset = |add: u8| {
            std::net::Ipv4Addr::from(u32::from(base).saturating_add(u32::from(add))).to_string()
        };
        networks.connector = offset(2);
        networks.entry = offset(3);
        self.ensure_network(&networks.relay, None, false).await?;
        Ok((project, networks))
    }

    /// 建一张由核心管的网络（不存在才建）。`subnet` 给了就固定网段；`isolated` 的网络两侧网关都关掉，
    /// 容器连不到宿主（ADR-0021“网络规则”）。
    async fn ensure_network(
        &self,
        name: &str,
        subnet: Option<&str>,
        isolated: bool,
    ) -> Result<Option<String>, EntryError> {
        if let Some(found) = self.engine.inspect_network(name).await? {
            return Ok(found
                .ipam
                .and_then(|ipam| ipam.config)
                .and_then(|config| config.first().cloned())
                .and_then(|config| config.subnet));
        }
        let mut labels = self.instance.core_labels();
        labels.insert(LABEL_MANAGED_BY.to_owned(), "core".to_owned());
        labels.insert(LABEL_COMPONENT.to_owned(), ENTRY_COMPONENT.to_owned());
        let request = bollard::models::NetworkCreateRequest {
            name: name.to_owned(),
            driver: Some("bridge".to_owned()),
            labels: Some(labels),
            internal: Some(isolated),
            options: isolated.then(|| {
                [
                    ("com.docker.network.bridge.gateway_mode_ipv4", "isolated"),
                    ("com.docker.network.bridge.gateway_mode_ipv6", "isolated"),
                ]
                .map(|(key, value)| (key.to_owned(), value.to_owned()))
                .into()
            }),
            ipam: subnet.map(|subnet| bollard::models::Ipam {
                config: Some(vec![bollard::models::IpamConfig {
                    subnet: Some(subnet.to_owned()),
                    ..Default::default()
                }]),
                ..Default::default()
            }),
            ..Default::default()
        };
        match self.engine.create_network_from(request.clone()).await {
            Ok(()) => {}
            // 同一台机器上有第二套云吞时默认网段被占：改用 Docker 的默认地址池挑一个空闲网段。
            Err(EngineError::Rejected { message, .. })
                if subnet.is_some() && message.contains("Pool overlaps") =>
            {
                let retry = NetworkCreateRequest {
                    ipam: None,
                    ..request
                };
                self.engine.create_network_from(retry).await?;
            }
            Err(error) => return Err(error.into()),
        }
        Ok(self
            .engine
            .inspect_network(name)
            .await?
            .and_then(|found| found.ipam)
            .and_then(|ipam| ipam.config)
            .and_then(|config| config.first().cloned())
            .and_then(|config| config.subnet))
    }

    /// 创建一侧的容器：已经在了就只补齐网络；镜像或运行参数变了、或者是上一版 compose 留下的同名容器，删了重建；
    /// 名字被别的资源占着就停。
    async fn ensure(&self, side: &Side, project: &str) -> Result<(), EntryError> {
        let name = self.name(side.component);
        let found = self.engine.inspect_container(&name).await?;
        let mut previous_version = false;
        if let Some(info) = &found {
            let labels = info
                .config
                .as_ref()
                .and_then(|config| config.labels.clone())
                .unwrap_or_default();
            if !self.owns(&labels, side.component, project) {
                return Err(EntryError::NameTaken(name));
            }
            previous_version = is_previous_version(&labels, side.component, project);
        }
        // 拉不到镜像时不动正在跑的容器。
        let image = self
            .engine
            .ensure_image(side.image, PullPolicy::Missing, &[])
            .await?;
        let current = found.as_ref().is_some_and(|info| {
            info.config
                .as_ref()
                .and_then(|config| config.image.as_deref())
                .is_some_and(|reference| {
                    crate::plugin::registry_mirror::reference::same_image(reference, side.image)
                })
        });
        let running = found
            .as_ref()
            .and_then(|info| info.state.as_ref())
            .and_then(|state| state.running)
            == Some(true);
        let mut body = side.body.clone();
        body.image = Some(image);
        let same_options = found.as_ref().is_some_and(|info| same_options(info, &body));
        let recreate = !current || !running || !same_options || previous_version;
        if found.is_some() && recreate {
            self.engine.remove_container(&name).await?;
        }
        if recreate {
            self.engine.create_container(&name, body).await?;
        }
        for (network, endpoint) in &side.endpoints {
            if self.engine.endpoint(&name, network).await?.is_none() {
                self.engine
                    .connect_network_at(network, &name, endpoint.clone())
                    .await?;
            }
        }
        if recreate {
            self.engine.start_container(&name).await?;
        }
        Ok(())
    }

    fn owns(&self, labels: &HashMap<String, String>, component: &str, project: &str) -> bool {
        self.instance.owns(labels)
            && (is_previous_version(labels, component, project)
                || labels.get(LABEL_MANAGED_BY).map(String::as_str) == Some("core")
                    && labels.get(LABEL_COMPONENT).map(String::as_str) == Some(component))
    }

    fn labels(&self, component: &str) -> HashMap<String, String> {
        let mut labels = self.instance.core_labels();
        labels.insert(LABEL_MANAGED_BY.to_owned(), "core".to_owned());
        labels.insert(LABEL_COMPONENT.to_owned(), component.to_owned());
        labels
    }

    /// 一个组件的全部创建参数。安全选项和主 compose 里原来那段一样（ADR-0021“网络规则”）：非 root、只读根、
    /// 能力全去、不升权、清代理变量；不挂任何 Docker socket。
    fn side(
        &self,
        component: &'static str,
        networks: &Networks,
        settings: &EntrySettings,
    ) -> Result<Side, EntryError> {
        let (image, command, binds, ports) = self.spec(component, settings)?;
        // 入口和连接器要解析的正是主人给的解析器认识的名字（中继、ACME）：compose 时代是核心和 lego 用这些解析器，
        // 容器自己的解析器跟着来，不然只能靠宿主的。
        let resolvers =
            crate::entry_settings::resolver_ips(&settings.acme_resolvers).map_err(|error| {
                EntryError::Docker(EngineError::Rejected {
                    context: "解析入口的 DNS 解析器".to_owned(),
                    message: error.to_string(),
                })
            })?;
        let port_bindings: HashMap<String, Option<Vec<PortBinding>>> = ports
            .iter()
            .map(|(key, host)| {
                let (host_ip, host_port) = host.rsplit_once(':').expect("写法是地址:端口");
                (
                    key.clone(),
                    Some(vec![PortBinding {
                        host_ip: Some(host_ip.to_owned()),
                        host_port: Some(host_port.to_owned()),
                    }]),
                )
            })
            .collect();
        let endpoints = endpoints(component, networks);
        // `network_mode: none` 的容器接不上别的网络（Docker 29 实测）：创建时就把端点写进 networking_config，
        // 主网络写 NetworkMode，其余端点一起带上。
        let primary = endpoints.first().expect("每个组件都有网络").0.clone();
        let endpoints_config: HashMap<String, EndpointSettings> = endpoints
            .iter()
            .map(|(network, endpoint)| {
                (
                    network.clone(),
                    EndpointSettings {
                        aliases: Some(endpoint.aliases.clone()),
                        gw_priority: endpoint.gw_priority,
                        ipam_config: endpoint.address.clone().map(|address| EndpointIpamConfig {
                            ipv4_address: Some(address),
                            ..Default::default()
                        }),
                        ..Default::default()
                    },
                )
            })
            .collect();
        Ok(Side {
            component,
            image,
            body: ContainerCreateBody {
                user: Some(self.user()?),
                cmd: Some(command),
                env: Some(
                    ["HTTP_PROXY", "HTTPS_PROXY", "http_proxy", "https_proxy"]
                        .into_iter()
                        .map(|name| format!("{name}="))
                        .collect(),
                ),
                labels: Some(self.labels(component)),
                exposed_ports: (!ports.is_empty())
                    .then(|| ports.iter().map(|(key, _)| key.clone()).collect()),
                networking_config: Some(NetworkingConfig {
                    endpoints_config: Some(endpoints_config),
                }),
                host_config: Some(HostConfig {
                    binds: Some(binds),
                    network_mode: Some(primary),
                    dns: (!resolvers.is_empty()).then_some(resolvers),
                    restart_policy: Some(RestartPolicy {
                        name: Some(RestartPolicyNameEnum::UNLESS_STOPPED),
                        ..Default::default()
                    }),
                    cap_drop: Some(vec!["ALL".to_owned()]),
                    security_opt: Some(vec!["no-new-privileges:true".to_owned()]),
                    readonly_rootfs: Some(true),
                    port_bindings: (!ports.is_empty()).then_some(port_bindings),
                    log_config: Some(HostConfigLogConfig {
                        typ: Some("json-file".to_owned()),
                        config: Some(HashMap::from([
                            ("max-size".to_owned(), "10m".to_owned()),
                            ("max-file".to_owned(), "3".to_owned()),
                        ])),
                    }),
                    ..Default::default()
                }),
                ..Default::default()
            },
            endpoints,
        })
    }

    /// 镜像、命令行、只读挂载和要发布的端口（`容器端口` → `宿主地址:端口`）。
    #[allow(clippy::type_complexity)] // 四项各自都不是同一个东西，起名反而看不清。
    fn spec(
        &self,
        component: &str,
        settings: &EntrySettings,
    ) -> Result<
        (
            &'static str,
            Vec<String>,
            Vec<String>,
            Vec<(String, String)>,
        ),
        EntryError,
    > {
        let entry = self.install_dir.join("entry");
        Ok(if component == CONNECTOR_COMPONENT {
            match &settings.registration {
                Some(_) => (
                    crate::entry_runtime::FRPC_IMAGE,
                    vec!["-c".to_owned(), "/config/frpc.json".to_owned()],
                    mounts(
                        &entry,
                        &[
                            ("frpc.json", "/config/frpc.json"),
                            ("relay-ca.pem", "/config/relay-ca.pem"),
                        ],
                    ),
                    Vec::new(),
                ),
                None => (
                    crate::entry_compose::HAPROXY_IMAGE,
                    haproxy_command("/config/connector.cfg"),
                    mounts(&entry, &[("connector.cfg", "/config/connector.cfg")]),
                    // 路由器把公网端口转到宿主，连接器在宿主上收。
                    vec![(format!("{CONNECTOR_PORT}/tcp"), self.host_binding())],
                ),
            }
        } else {
            (
                crate::entry_compose::HAPROXY_IMAGE,
                haproxy_command("/config/haproxy.cfg"),
                mounts(
                    &entry,
                    &[("haproxy.cfg", "/config/haproxy.cfg"), ("certs", "/certs")],
                ),
                Vec::new(),
            )
        })
    }

    /// `edge` 的网段和装机时不一样：按实际地址重写入口配置（连接器地址写在 TLS 前端的对端条件里）。
    fn rewrite_config(&self, settings: &EntrySettings, connector: &str) -> Result<(), EntryError> {
        let mut model =
            crate::entry_haproxy::Model::federation_only(&settings.domain, &settings.domain);
        model.connectors = vec![connector.to_owned()];
        crate::entry_compose::write_model(&self.install_dir, &model).map_err(|error| {
            EntryError::Docker(EngineError::Rejected {
                context: "重写入口配置".to_owned(),
                message: error.to_string(),
            })
        })?;
        crate::entry_compose::prepare_permissions(&self.install_dir).map_err(|error| {
            EntryError::Docker(EngineError::NotFound(format!("入口文件权限：{error}")))
        })
    }

    /// 运行的用户跟证书的属主走：证书可能是主人自己放的（`WONTON_ENTRY_CERTIFICATE=external`）。
    fn user(&self) -> Result<String, EntryError> {
        let path = self.install_dir.join("entry/certs/site.pem");
        let cert = std::fs::metadata(&path).map_err(|error| {
            EntryError::Docker(EngineError::NotFound(format!(
                "{} 读不了：{error}",
                path.display()
            )))
        })?;
        Ok(format!("{}:{}", cert.uid(), cert.gid()))
    }

    /// 连接器发布在宿主哪里：`WONTON_ENTRY_BIND`、`WONTON_ENTRY_PORT` 可改，默认只有回环。
    fn host_binding(&self) -> String {
        let (bind, port) = self.binding.clone().unwrap_or_else(|| {
            let bind = std::env::var("WONTON_ENTRY_BIND")
                .ok()
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| "127.0.0.1".to_owned());
            let port = std::env::var("WONTON_ENTRY_PORT")
                .ok()
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| CONNECTOR_PORT.to_string());
            (bind, port)
        });
        // IPv6 的写法是 `[::1]:8443`，地址里的冒号不能当作分隔符。
        let bind = bind.trim_start_matches('[').trim_end_matches(']');
        format!("{bind}:{port}")
    }
}

/// 接哪些网络：入口同时接 `clients` 供核心的 8484 用；连接器只在 `edge` 上被入口认，出网走 `relay`。
fn endpoints(component: &str, networks: &Networks) -> Vec<(String, Endpoint)> {
    if component == CONNECTOR_COMPONENT {
        return vec![
            (
                networks.edge.clone(),
                Endpoint {
                    address: Some(networks.connector.clone()),
                    aliases: vec![CONNECTOR_ALIAS.to_owned()],
                    // edge 是 internal：优先级压到最低，默认路由留在 relay 上。
                    gw_priority: Some(-1),
                },
            ),
            (networks.relay.clone(), Endpoint::default()),
        ];
    }
    vec![
        (
            networks.edge.clone(),
            Endpoint {
                address: Some(networks.entry.clone()),
                aliases: vec![ENTRY_EDGE_ALIAS.to_owned()],
                gw_priority: None,
            },
        ),
        (
            networks.clients.clone(),
            Endpoint {
                aliases: vec![ENTRY_CLIENTS_ALIAS.to_owned()],
                ..Endpoint::default()
            },
        ),
    ]
}

/// 上一版 compose 建的同名容器：只带 compose 的标签、没有我们的组件标签。升级到这条线以后第一次启动时
/// 它们还占着名字，升级器要等 `up` 末尾的 `--remove-orphans` 才删，这里直接认领、删了重建。
fn is_previous_version(labels: &HashMap<String, String>, component: &str, project: &str) -> bool {
    labels.get("com.docker.compose.project").map(String::as_str) == Some(project)
        && labels.get("com.docker.compose.service").map(String::as_str) == Some(component)
        && labels.get(LABEL_MANAGED_BY).is_none()
}

/// 运行参数和期望的一样：用户、只读挂载、发布端口、代理变量。不一样就重建（改了端口要生效）。
fn same_options(info: &ContainerInspectResponse, body: &ContainerCreateBody) -> bool {
    let config = info.config.as_ref();
    let host = info.host_config.as_ref();
    let (Some(config), Some(host)) = (config, host) else {
        return false;
    };
    if config.user != body.user || host.dns != body.host_config.as_ref().and_then(|h| h.dns.clone())
    {
        return false;
    }
    let wanted_host = body.host_config.as_ref();
    let binds = |list: Option<&Vec<String>>| list.cloned().unwrap_or_default();
    if binds(host.binds.as_ref()) != binds(wanted_host.and_then(|h| h.binds.as_ref())) {
        return false;
    }
    let ports =
        |map: Option<&HashMap<String, Option<Vec<PortBinding>>>>| map.cloned().unwrap_or_default();
    if ports(host.port_bindings.as_ref())
        != ports(wanted_host.and_then(|h| h.port_bindings.as_ref()))
    {
        return false;
    }
    // 容器的环境变量里还有镜像自己的（PATH 之类），只要求我们那几项都在。
    let have: std::collections::BTreeSet<&str> = config
        .env
        .as_ref()
        .map(|env| env.iter().map(String::as_str).collect())
        .unwrap_or_default();
    body.env
        .as_ref()
        .is_none_or(|env| env.iter().all(|item| have.contains(item.as_str())))
}

/// 容器里只读挂的配置文件。
fn mounts(entry: &Path, names: &[(&str, &str)]) -> Vec<String> {
    names
        .iter()
        .map(|(file, mount)| {
            let source = entry.join(file);
            format!("{}:{mount}:ro", source.display())
        })
        .collect()
}

fn haproxy_command(config: &str) -> Vec<String> {
    ["haproxy", "-W", "-db", "-f", config]
        .map(str::to_owned)
        .to_vec()
}

struct Networks {
    clients: String,
    edge: String,
    relay: String,
    /// 入口和连接器在 `edge` 上的固定地址：网段基址 +2/+3（默认网段就是
    /// [`CONNECTOR_ADDRESS`]/[`ENTRY_EDGE_ADDRESS`]）。
    connector: String,
    entry: String,
}

#[cfg(test)]
mod tests;
