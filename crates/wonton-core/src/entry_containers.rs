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
    ContainerCreateBody, HostConfig, HostConfigLogConfig, PortBinding, RestartPolicy,
    RestartPolicyNameEnum,
};

use crate::entry_haproxy::{CONNECTOR_ADDRESS, ENTRY_EDGE_ADDRESS};
use crate::entry_settings::EntrySettings;
use crate::plugin::compose::PullPolicy;
use crate::plugin::core_address::Endpoint;
use crate::plugin::docker::{Engine, EngineError};
use crate::plugin::names::{Instance, LABEL_COMPONENT, LABEL_MANAGED_BY};

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
            Self::CoreNotInContainer => "core_not_in_container",
            Self::NameTaken(_) | Self::NotOneEntry(_) => "conflict",
            Self::NetworkMissing(_) => "core_not_in_container",
            Self::Docker(_) => "docker_unavailable",
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
        }
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

    /// 有联邦或者任何一个打开的入口就要入口容器；有连接器设置就要连接器（C5 里是家名服务和路由器转发的默认端口）。
    /// 没有设置就两个都不要，连网络也不用查。
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
        let mut networks = None;
        let sides: [(bool, &'static str); 2] =
            [(entry, ENTRY_COMPONENT), (connector, CONNECTOR_COMPONENT)];
        for (wanted, component) in sides {
            let name = self.name(component);
            if !wanted {
                self.engine.remove_container(&name).await?;
                continue;
            }
            if networks.is_none() {
                networks = Some(self.networks(core).await?);
            }
            let networks = networks.as_ref().expect("网络刚取过");
            let side = self.side(component, networks, settings.expect("要有设置才可能有容器"))?;
            self.ensure(&side).await?;
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

    /// compose 给主项目渲染的网络名：`<项目>_clients`。项目名从核心自己的标签读，开发栈和正式安装不写死。
    async fn networks(&self, core: &str) -> Result<Networks, EntryError> {
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
        let networks = Networks {
            clients: network("clients"),
            edge: network("edge"),
            relay: network("relay"),
        };
        for name in [&networks.clients, &networks.edge, &networks.relay] {
            if self.engine.inspect_network(name).await?.is_none() {
                return Err(EntryError::NetworkMissing(name.clone()));
            }
        }
        Ok(networks)
    }

    /// 创建一侧的容器：已经在了就只补齐网络；镜像变了重建；名字被别的资源占着就停。
    async fn ensure(&self, side: &Side) -> Result<(), EntryError> {
        let name = self.name(side.component);
        let found = self.engine.inspect_container(&name).await?;
        if let Some(info) = &found {
            let labels = info
                .config
                .as_ref()
                .and_then(|config| config.labels.clone())
                .unwrap_or_default();
            if !self.owns(&labels, side.component) {
                return Err(EntryError::NameTaken(name));
            }
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
        if found.is_some() && (!current || !running) {
            self.engine.remove_container(&name).await?;
        }
        if !current || !running {
            self.engine.create_container(&name, body).await?;
        }
        for (network, endpoint) in &side.endpoints {
            if self.engine.endpoint(&name, network).await?.is_none() {
                self.engine
                    .connect_network_at(network, &name, endpoint.clone())
                    .await?;
            }
        }
        if !current || !running {
            self.engine.start_container(&name).await?;
        }
        Ok(())
    }

    fn owns(&self, labels: &HashMap<String, String>, component: &str) -> bool {
        self.instance.owns(labels)
            && labels.get(LABEL_MANAGED_BY).map(String::as_str) == Some("core")
            && labels.get(LABEL_COMPONENT).map(String::as_str) == Some(component)
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
                host_config: Some(HostConfig {
                    binds: Some(binds),
                    network_mode: Some("none".to_owned()),
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
            endpoints: endpoints(component, networks),
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
                    vec![(format!("{CONNECTOR_PORT}/tcp"), host_binding())],
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
}

/// 接哪些网络：入口同时接 `clients` 供核心的 8484 用；连接器只在 `edge` 上被入口认，出网走 `relay`。
fn endpoints(component: &str, networks: &Networks) -> Vec<(String, Endpoint)> {
    if component == CONNECTOR_COMPONENT {
        return vec![
            (
                networks.edge.clone(),
                Endpoint {
                    address: Some(CONNECTOR_ADDRESS.to_owned()),
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
                address: Some(ENTRY_EDGE_ADDRESS.to_owned()),
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
}

/// 连接器发布在宿主哪里：`WONTON_ENTRY_BIND`、`WONTON_ENTRY_PORT` 可改，默认只有回环。
fn host_binding() -> String {
    let bind = std::env::var("WONTON_ENTRY_BIND")
        .ok()
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "127.0.0.1".to_owned());
    // IPv6 的写法是 `[::1]:8443`，地址里的冒号不能当作分隔符。
    let bind = bind
        .trim_start_matches('[')
        .trim_end_matches(']')
        .to_owned();
    let port = std::env::var("WONTON_ENTRY_PORT")
        .ok()
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| CONNECTOR_PORT.to_string());
    format!("{bind}:{port}")
}

#[cfg(test)]
mod tests;
