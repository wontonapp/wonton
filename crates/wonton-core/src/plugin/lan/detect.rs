//! 检测宿主的网络（ADR-0019 的“怎么检测”）。核心在容器里看不到宿主的网卡，所以用自己的镜像起一个一次性的
//! 小容器跑 [`super::probe`]：宿主网络，去掉全部能力，根文件系统只读，不挂任何东西，以 nobody 身份运行；跑完就删，
//! 失败了也删。核心直接跑在宿主上时（开发时）直接读。

use std::collections::HashMap;
use std::path::Path;
#[cfg(test)]
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bollard::container::LogOutput;
use bollard::errors::Error as BollardError;
use bollard::models::{ContainerCreateBody, HostConfig, NetworkCreateRequest};
use bollard::query_parameters::LogsOptionsBuilder;
use futures_util::StreamExt;

use super::probe::{self, HostNetwork};
use super::select::DockerBridges;
use crate::plugin::docker::{Engine, EngineError};

/// 检测容器最多跑这么久：它只读几个文件、发一次 netlink 请求，正常不到一秒。
const PROBE_TIMEOUT: Duration = Duration::from_secs(30);

const MASQUERADE: &str = "com.docker.network.bridge.enable_ip_masquerade";

/// 怎么检测宿主的网络。
#[derive(Clone)]
pub enum Detector {
    /// 核心在容器里（`WONTON_CORE_CONTAINER`）：用这个容器的镜像起检测容器。
    Container { core: String },
    /// 核心直接跑在宿主上：直接读。
    Here,
    /// 测试用：检测结果由测试给出，可以中途改。
    #[cfg(test)]
    Fixed(Arc<Mutex<Result<HostNetwork, String>>>),
}

impl std::fmt::Debug for Detector {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Container { core } => write!(f, "Container({core})"),
            Self::Here => f.write_str("Here"),
            #[cfg(test)]
            Self::Fixed(_) => f.write_str("Fixed"),
        }
    }
}

impl Detector {
    /// 检测一次。Docker 连不上时报错；检测本身失败（检测容器起不来、退出码不是 0、输出不对）时为 `Ok(Err(原因))`。
    /// `name` 和 `labels` 是检测容器的名字和标签。
    pub async fn detect(
        &self,
        engine: &Engine,
        name: &str,
        labels: HashMap<String, String>,
    ) -> Result<Result<HostNetwork, String>, EngineError> {
        match self {
            Self::Container { core } => engine.run_probe(core, name, labels).await,
            Self::Here => Ok(tokio::task::spawn_blocking(|| probe::probe(Path::new("/")))
                .await
                .unwrap_or_else(|e| Err(format!("检测宿主的网卡时出错：{e}")))),
            #[cfg(test)]
            Self::Fixed(network) => Ok(network.lock().expect("检测结果锁中毒").clone()),
        }
    }
}

impl Engine {
    /// 用核心容器的镜像起检测容器，读它打印的结果。不论成败都删掉它；开始前先删掉上次核心在检测中途退出留下的。
    async fn run_probe(
        &self,
        core: &str,
        name: &str,
        labels: HashMap<String, String>,
    ) -> Result<Result<HostNetwork, String>, EngineError> {
        let Some(image) = self.inspect_container(core).await?.and_then(|c| c.image) else {
            return Ok(Err(format!("查不到核心容器 {core} 的镜像")));
        };
        self.remove_container(name).await?;
        let body = ContainerCreateBody {
            image: Some(image),
            cmd: Some(vec![probe::COMMAND.to_owned()]),
            user: Some("65534:65534".to_owned()),
            labels: Some(labels),
            host_config: Some(HostConfig {
                network_mode: Some("host".to_owned()),
                cap_drop: Some(vec!["ALL".to_owned()]),
                security_opt: Some(vec!["no-new-privileges".to_owned()]),
                readonly_rootfs: Some(true),
                ..Default::default()
            }),
            ..Default::default()
        };
        if let Err(e) = self.create_container(name, body).await {
            return unless_unavailable(e);
        }
        let result = self.finish_probe(name).await;
        let removed = self.remove_container(name).await;
        let network = result?;
        removed?;
        Ok(network)
    }

    async fn finish_probe(&self, name: &str) -> Result<Result<HostNetwork, String>, EngineError> {
        if let Err(e) = self.start_container(name).await {
            return unless_unavailable(e);
        }
        let mut wait = self.docker.wait_container(name, None);
        let code = match tokio::time::timeout(PROBE_TIMEOUT, wait.next()).await {
            Err(_) => {
                let seconds = PROBE_TIMEOUT.as_secs();
                return Ok(Err(format!("检测容器 {seconds} 秒没跑完")));
            }
            Ok(None) => return Ok(Err("Docker 没给出检测容器的退出码".to_owned())),
            Ok(Some(Ok(exit))) => exit.status_code,
            // bollard 把非 0 的退出码当成错误交回来。
            Ok(Some(Err(BollardError::DockerContainerWaitError { code, .. }))) => code,
            Ok(Some(Err(e))) => return Err(EngineError::from_bollard("等检测容器退出", e)),
        };
        let options = LogsOptionsBuilder::default()
            .stdout(true)
            .stderr(true)
            .build();
        let mut logs = self.docker.logs(name, Some(options));
        let (mut stdout, mut stderr) = (Vec::new(), Vec::new());
        while let Some(chunk) = logs.next().await {
            match chunk.map_err(|e| EngineError::from_bollard("读检测容器的输出", e))? {
                LogOutput::StdOut { message } => stdout.extend_from_slice(&message),
                other => stderr.extend_from_slice(other.as_ref()),
            }
        }
        if code != 0 {
            let reason = String::from_utf8_lossy(&stderr);
            return Ok(Err(format!("检测容器以 {code} 退出：{}", reason.trim())));
        }
        Ok(serde_json::from_slice(&stdout).map_err(|e| format!("检测容器的输出不对：{e}")))
    }

    /// Docker 的网桥：按 Docker 的网络列表认，不按网卡名字的样子猜。
    pub(in crate::plugin) async fn bridges(&self) -> Result<DockerBridges, EngineError> {
        let networks = self
            .docker
            .list_networks(None)
            .await
            .map_err(|e| EngineError::from_bollard("列出网络", e))?;
        let mut bridges = DockerBridges::default();
        for network in networks {
            if network.driver.as_deref() != Some("bridge") {
                continue;
            }
            // 没指定网桥名时，Docker 用 `br-` 加网络 ID 的前 12 位。
            let named = network
                .options
                .as_ref()
                .and_then(|o| o.get("com.docker.network.bridge.name").cloned());
            let derived = network
                .id
                .as_ref()
                .map(|id| format!("br-{}", id.get(..12).unwrap_or(id)));
            bridges.names.extend(named.or(derived));
            let configs = network.ipam.and_then(|i| i.config).unwrap_or_default();
            let gateways = configs
                .into_iter()
                .filter_map(|c| c.gateway?.parse::<std::net::IpAddr>().ok());
            bridges.gateways.extend(gateways);
        }
        Ok(bridges)
    }

    /// 发布局域网端口的网络：普通桥接，关掉地址伪装。接进来的服务主动连外网或局域网的其他主机都收不到回包。
    pub(in crate::plugin) async fn create_lan_network(
        &self,
        name: &str,
        labels: &HashMap<String, String>,
    ) -> Result<(), EngineError> {
        let request = NetworkCreateRequest {
            name: name.to_owned(),
            driver: Some("bridge".to_owned()),
            labels: Some(labels.clone()),
            internal: Some(false),
            options: Some(HashMap::from([(MASQUERADE.to_owned(), "false".to_owned())])),
            ..Default::default()
        };
        self.docker
            .create_network(request)
            .await
            .map(|_| ())
            .map_err(|e| EngineError::from_bollard(&format!("创建网络 {name}"), e))
    }
}

/// Docker 连不上照常报错；Docker 拒绝了（镜像没了、宿主不让用宿主网络等）算检测失败。
fn unless_unavailable(e: EngineError) -> Result<Result<HostNetwork, String>, EngineError> {
    match e {
        EngineError::Unavailable(_) => Err(e),
        other => Ok(Err(other.to_string())),
    }
}
