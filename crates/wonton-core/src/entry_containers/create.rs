//! 容器创建：镜像、命令行、只读挂载、安全选项和端点，逐项和主 compose 原来那段保持一致。

use std::collections::HashMap;
use std::os::unix::fs::MetadataExt;
use std::path::Path;

use bollard::models::{
    ContainerCreateBody, EndpointIpamConfig, EndpointSettings, HostConfig, HostConfigLogConfig,
    NetworkingConfig, PortBinding, RestartPolicy, RestartPolicyNameEnum,
};

use super::network::{Networks, endpoints};
use super::target::Target;
use super::{
    CONNECTOR_COMPONENT, CONNECTOR_PORT, CONNECTOR_RELOAD_ADDR, ENTRY_RELOAD_ADDR, EntryContainers,
    EntryError,
};
use crate::plugin::core_address::Endpoint;
use crate::plugin::docker::EngineError;
use crate::plugin::names::{LABEL_COMPONENT, LABEL_MANAGED_BY};

/// 一侧容器的期望状态：接哪些网络、用什么镜像。
pub(super) struct Side {
    pub(super) component: &'static str,
    pub(super) image: &'static str,
    pub(super) body: ContainerCreateBody,
    /// 接进的网络、固定地址、别名和网关优先级。
    pub(super) endpoints: Vec<(String, Endpoint)>,
}

impl EntryContainers {
    /// 一个组件的全部创建参数。安全选项和主 compose 里原来那段一样（ADR-0021“网络规则”）：非 root、只读根、
    /// 能力全去、不升权、清代理变量；不挂任何 Docker socket。
    pub(super) fn side(
        &self,
        component: &'static str,
        networks: &Networks,
        target: &Target,
    ) -> Result<Side, EntryError> {
        let (image, command, binds, ports) = self.spec(component, target)?;
        // 入口和连接器要解析的正是主人给的解析器认识的名字（中继、ACME）：compose 时代是核心和 lego 用这些解析器，
        // 容器自己的解析器跟着来，不然只能靠宿主的。
        let resolvers =
            crate::entry_settings::resolver_ips(&target.resolvers).map_err(|error| {
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
                env: Some({
                    let mut env: Vec<String> =
                        ["HTTP_PROXY", "HTTPS_PROXY", "http_proxy", "https_proxy"]
                            .into_iter()
                            .map(|name| format!("{name}="))
                            .collect();
                    // frpc 只在启动时读配置：把配置内容摘要放进环境变量，内容一变 `same_options` 就判重建。
                    if component == CONNECTOR_COMPONENT {
                        let name = if target.registration.is_some() {
                            "frpc.json"
                        } else {
                            "connector.cfg"
                        };
                        if let Ok(bytes) = std::fs::read(self.install_dir.join("entry").join(name))
                        {
                            use std::hash::{Hash, Hasher};
                            let mut hasher = std::collections::hash_map::DefaultHasher::new();
                            bytes.hash(&mut hasher);
                            env.push(format!("WONTON_CONNECTOR_SPEC={:016x}", hasher.finish()));
                        }
                    }
                    env
                }),
                labels: Some(self.labels(component)),
                // Docker 重启时停容器用它。不写就跟镜像的 StopSignal 走，镜像改了可能来不及排空连接。
                stop_signal: Some("SIGTERM".to_owned()),
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
                        name: Some(RestartPolicyNameEnum::ALWAYS),
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
        target: &Target,
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
            match &target.registration {
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
                    haproxy_command("/config/connector.cfg", CONNECTOR_RELOAD_ADDR),
                    mounts(&entry, &[("connector.cfg", "/config/connector.cfg")]),
                    // 路由器把公网端口转到宿主，连接器在宿主上收。
                    vec![(format!("{CONNECTOR_PORT}/tcp"), self.host_binding())],
                ),
            }
        } else {
            (
                crate::entry_compose::HAPROXY_IMAGE,
                haproxy_command("/config/haproxy.cfg", ENTRY_RELOAD_ADDR),
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

    fn labels(&self, component: &str) -> HashMap<String, String> {
        let mut labels = self.instance.core_labels();
        labels.insert(LABEL_MANAGED_BY.to_owned(), "core".to_owned());
        labels.insert(LABEL_COMPONENT.to_owned(), component.to_owned());
        labels
    }
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

/// `-S` 开主进程的管理端口，热加载走它；`-W` 是主从模式，新配置起不来时旧进程照常跑。
fn haproxy_command(config: &str, reload_addr: &str) -> Vec<String> {
    ["haproxy", "-W", "-db", "-f", config, "-S", reload_addr]
        .map(str::to_owned)
        .to_vec()
}
