//! 检测宿主的网络（ADR-0019 的“怎么检测”）。核心在容器里看不到宿主的网卡，所以用自己的镜像起一个一次性的
//! 小容器跑 [`super::probe`]：宿主网络，去掉全部能力，根文件系统只读，不挂任何东西，以 nobody 身份运行；跑完就删，
//! 失败了也删。核心直接跑在宿主上时（开发时）直接读。

use std::collections::HashMap;
use std::path::Path;
#[cfg(test)]
use std::sync::{Arc, Mutex};
use std::time::Duration;

use std::net::IpAddr;

use bollard::container::LogOutput;
use bollard::errors::Error as BollardError;
use bollard::models::{ContainerCreateBody, HostConfig, NetworkCreateRequest};
use bollard::query_parameters::LogsOptionsBuilder;
use futures_util::StreamExt;
use ipnet::Ipv4Net;

use super::probe::{self, HostNetwork};
use super::select::DockerBridges;
use crate::l10n;
use crate::plugin::docker::{Engine, EngineError};
use crate::plugin::languages::Languages;

/// 一次性容器最多跑这么久：检测容器只读几个文件、发一次 netlink 请求，正常不到一秒；查代理的最多 20 秒。
const PROBE_TIMEOUT: Duration = Duration::from_secs(30);

const MASQUERADE: &str = "com.docker.network.bridge.enable_ip_masquerade";

/// 一次性容器跑完的结果。
pub(in crate::plugin) struct OneShot {
    pub code: i64,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

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
    /// 测试用：检测结果由测试给出，宿主端口探测总是失败（看预览有没有如实写明）。
    #[cfg(test)]
    FixedListen(Arc<Mutex<Result<HostNetwork, String>>>, String),
}

impl std::fmt::Debug for Detector {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Container { core } => write!(f, "Container({core})"),
            Self::Here => f.write_str("Here"),
            #[cfg(test)]
            Self::Fixed(_) => f.write_str("Fixed"),
            #[cfg(test)]
            Self::FixedListen(..) => f.write_str("FixedListen"),
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
            Self::Container { core } => Ok(
                match engine
                    .run_probe(core, name, labels, vec![probe::COMMAND.to_owned()])
                    .await?
                {
                    ProbeRun::Failed(reason) => Err(reason),
                    ProbeRun::Ran { stdout, replaced } => {
                        serde_json::from_slice(&stdout).map_err(|e| {
                            let langs = engine.owner_languages();
                            let reason = l10n::text(
                                &langs,
                                "plugin-settings-lan-detect-output",
                                &[("error", e.to_string().into())],
                            );
                            with_replaced_note(&langs, replaced, reason)
                        })
                    }
                },
            ),
            Self::Here => {
                let langs = engine.owner_languages();
                Ok(tokio::task::spawn_blocking(|| probe::probe(Path::new("/")))
                    .await
                    .unwrap_or_else(|e| {
                        Err(l10n::text(
                            &langs,
                            "plugin-settings-lan-detect-error",
                            &[("error", e.to_string().into())],
                        ))
                    }))
            }
            #[cfg(test)]
            Self::Fixed(network) => Ok(network.lock().expect("检测结果锁中毒").clone()),
            #[cfg(test)]
            Self::FixedListen(network, _) => Ok(network.lock().expect("检测结果锁中毒").clone()),
        }
    }

    /// 这些地址和端口在宿主上有程序在听吗（预览用，宿主上的非 Docker 进程占的端口 Docker 看不到）。返回听见的那些；
    /// 探测本身失败时是 `Err(原因)`，调用方别当成“没人占”。只判得了 TCP，调用方只传 TCP 端口。
    pub async fn listen(
        &self,
        engine: &Engine,
        name: &str,
        labels: HashMap<String, String>,
        targets: &[(IpAddr, u16)],
    ) -> Result<Vec<(IpAddr, u16)>, String> {
        if targets.is_empty() {
            return Ok(Vec::new());
        }
        let list = targets
            .iter()
            .map(|(address, port)| match address {
                IpAddr::V6(_) => format!("[{address}]:{port}"),
                IpAddr::V4(_) => format!("{address}:{port}"),
            })
            .collect::<Vec<_>>()
            .join(",");
        match self {
            Self::Container { core } => {
                let run = engine
                    .run_probe(
                        core,
                        name,
                        labels,
                        vec![probe::COMMAND.to_owned(), probe::LISTEN.to_owned(), list],
                    )
                    .await
                    .map_err(|e| e.to_string())?;
                let stdout = match run {
                    ProbeRun::Failed(reason) => return Err(reason),
                    ProbeRun::Ran { stdout, replaced } => (stdout, replaced),
                };
                serde_json::from_slice(&stdout.0).map_err(|e| {
                    let langs = engine.owner_languages();
                    let reason = l10n::text(
                        &langs,
                        "plugin-settings-lan-detect-output",
                        &[("error", e.to_string().into())],
                    );
                    with_replaced_note(&langs, stdout.1, reason)
                })
            }
            Self::Here => {
                let targets = targets.to_vec();
                tokio::task::spawn_blocking(move || probe::listen(&targets))
                    .await
                    .map_err(|e| {
                        l10n::text(
                            &engine.owner_languages(),
                            "plugin-settings-lan-listen-error",
                            &[("error", e.to_string().into())],
                        )
                    })
            }
            #[cfg(test)]
            Self::Fixed(_) => Ok(Vec::new()),
            #[cfg(test)]
            Self::FixedListen(network, error) => match &*network.lock().expect("检测结果锁中毒")
            {
                Ok(_) => Err(error.clone()),
                Err(reason) => Err(reason.clone()),
            },
        }
    }
}

/// 一次性容器跑成的结果：输出，和起容器用的是不是核心容器创建时的镜像引用。
pub(in crate::plugin) struct CoreOnce {
    pub output: OneShot,
    /// 起容器用的不是核心正在跑的镜像 ID，而是它创建时的镜像引用：核心的镜像已被替换（#666）。
    /// 输出读不了时上层要说明这一点。
    pub replaced: bool,
}

/// 检测容器跑出来的东西：跑成了是原始标准输出（两种子命令各自解析），没跑成是原因。
enum ProbeRun {
    /// 跑成了。`replaced` 见 [`CoreOnce::replaced`]。
    Ran { stdout: Vec<u8>, replaced: bool },
    /// 没跑成。
    Failed(String),
}

/// Docker 拒绝创建容器、原话里是镜像查不到时为真。镜像查不到的原话是 `No such image: …`；
/// 网络查不到也是 404，但原话不同，不能当成镜像（见 `ports::taken`，按 Docker 的原话分类是惯例）。
fn image_missing(rejected: &EngineError) -> bool {
    matches!(rejected, EngineError::NotFound(message) if message.contains("No such image"))
}

/// 核心的镜像被换过时，在失败原因外面说明：镜像已被替换，重启家里的服务器就恢复（#666）。
pub(in crate::plugin) fn with_replaced_note(
    langs: &Languages,
    replaced: bool,
    reason: String,
) -> String {
    if replaced {
        l10n::text(
            langs,
            "plugin-install-probe-image-replaced",
            &[("reason", reason.into())],
        )
    } else {
        reason
    }
}

fn probe_body(
    image: &str,
    cmd: &[String],
    labels: &HashMap<String, String>,
) -> ContainerCreateBody {
    ContainerCreateBody {
        image: Some(image.to_owned()),
        cmd: Some(cmd.to_vec()),
        user: Some(probe::NOBODY_USER.to_owned()),
        labels: Some(labels.clone()),
        host_config: Some(HostConfig {
            network_mode: Some("host".to_owned()),
            cap_drop: Some(vec!["ALL".to_owned()]),
            security_opt: Some(vec!["no-new-privileges".to_owned()]),
            readonly_rootfs: Some(true),
            ..Default::default()
        }),
        ..Default::default()
    }
}

impl Engine {
    /// 用核心容器的镜像起检测容器，读它打印的结果（原始标准输出，两种子命令各自解析）。
    async fn run_probe(
        &self,
        core: &str,
        name: &str,
        labels: HashMap<String, String>,
        cmd: Vec<String>,
    ) -> Result<ProbeRun, EngineError> {
        let Some(info) = self.inspect_container(core).await? else {
            return Ok(ProbeRun::Failed(l10n::text(
                &self.owner_languages(),
                "plugin-settings-lan-detect-core-image",
                &[("container", core.into())],
            )));
        };
        let Some(image) = info.image else {
            return Ok(ProbeRun::Failed(l10n::text(
                &self.owner_languages(),
                "plugin-settings-lan-detect-core-image",
                &[("container", core.into())],
            )));
        };
        let reference = info.config.and_then(|c| c.image);
        let langs = self.owner_languages();
        let once = match self
            .run_once(name, probe_body(&image, &cmd, &labels), reference)
            .await?
        {
            Ok(once) => once,
            Err(failure) => return Ok(ProbeRun::Failed(failure)),
        };
        if once.output.code != 0 {
            let reason = l10n::text(
                &langs,
                "plugin-settings-lan-detect-exit",
                &[
                    ("code", once.output.code.to_string().into()),
                    (
                        "reason",
                        String::from_utf8_lossy(&once.output.stderr).trim().into(),
                    ),
                ],
            );
            return Ok(ProbeRun::Failed(with_replaced_note(
                &langs,
                once.replaced,
                reason,
            )));
        }
        Ok(ProbeRun::Ran {
            stdout: once.output.stdout,
            replaced: once.replaced,
        })
    }

    /// 起一个一次性容器，等它跑完，读它的输出。不论成败都删掉它；开始前先删掉上次核心在中途退出留下的。
    /// 镜像用 `body` 里的（核心正在跑的镜像 ID）。Docker 拒绝创建、原话里是镜像查不到（containerd 镜像
    /// 存储下核心的镜像标签被移到新镜像、旧镜像记录又被清掉后就是这样，#666）、核心容器又记得创建时的
    /// 镜像引用时，退回按那个引用再创建一次——它通常已指向新版本的核心，输出读不了时上层如实说明。
    /// Docker 连不上时报错；Docker 拒绝了（镜像没了、宿主不让用宿主网络等）时为 `Err(原因)`，镜像被换过
    /// 的说明已经附在原因里。
    pub(in crate::plugin) async fn run_once(
        &self,
        name: &str,
        mut body: ContainerCreateBody,
        fallback: Option<String>,
    ) -> Result<Result<CoreOnce, String>, EngineError> {
        let _one_at_a_time = self.one_shot.lock().await;
        self.remove_container(name).await?;
        let mut replaced = false;
        if let Err(e) = self.create_container(name, body.clone()).await {
            let rejected = unless_unavailable(e)?;
            match (&rejected, &fallback) {
                (rejected, Some(reference)) if image_missing(rejected) => {
                    tracing::info!(
                        name,
                        image = %reference,
                        "核心的镜像 ID 起不了一次性容器，改用核心容器创建时的镜像引用"
                    );
                    body.image = Some(reference.clone());
                    replaced = true;
                    if let Err(e) = self.create_container(name, body).await {
                        let rejected = unless_unavailable(e)?;
                        return Ok(Err(l10n::text(
                            &self.owner_languages(),
                            "plugin-install-probe-image-missing",
                            &[("error", rejected.to_string().into())],
                        )));
                    }
                }
                _ => return Ok(Err(rejected.to_string())),
            }
        }
        let result = self.finish_once(name).await;
        let removed = self.remove_container(name).await;
        let output = result?;
        removed?;
        Ok(match output {
            Ok(output) => Ok(CoreOnce { output, replaced }),
            Err(failure) => Err(with_replaced_note(
                &self.owner_languages(),
                replaced,
                failure,
            )),
        })
    }

    async fn finish_once(&self, name: &str) -> Result<Result<OneShot, String>, EngineError> {
        if let Err(e) = self.start_container(name).await {
            return unless_unavailable(e).map(|rejected| Err(rejected.to_string()));
        }
        let mut wait = self.docker.wait_container(name, None);
        let code = match tokio::time::timeout(PROBE_TIMEOUT, wait.next()).await {
            Err(_) => {
                let seconds = PROBE_TIMEOUT.as_secs();
                let langs = self.owner_languages();
                return Ok(Err(crate::l10n::text(
                    &langs,
                    "plugin-settings-lan-detect-timeout",
                    &[
                        ("container", name.into()),
                        ("seconds", seconds.to_string().into()),
                    ],
                )));
            }
            Ok(None) => {
                let langs = self.owner_languages();
                return Ok(Err(crate::l10n::text(
                    &langs,
                    "plugin-settings-lan-detect-no-exit-code",
                    &[("container", name.into())],
                )));
            }
            Ok(Some(Ok(exit))) => exit.status_code,
            // bollard 把非 0 的退出码当成错误交回来。
            Ok(Some(Err(BollardError::DockerContainerWaitError { code, .. }))) => code,
            Ok(Some(Err(e))) => {
                let langs = self.owner_languages();
                return Err(crate::plugin::docker::from_bollard(
                    &langs,
                    &crate::l10n::text(&langs, "plugin-settings-lan-detect-wait-exit", &[]),
                    e,
                ));
            }
        };
        let options = LogsOptionsBuilder::default()
            .stdout(true)
            .stderr(true)
            .build();
        let mut logs = self.docker.logs(name, Some(options));
        let (mut stdout, mut stderr) = (Vec::new(), Vec::new());
        while let Some(chunk) = logs.next().await {
            match chunk.map_err(|e| {
                let langs = self.owner_languages();
                crate::plugin::docker::from_bollard(
                    &langs,
                    &crate::l10n::text(&langs, "plugin-settings-lan-detect-read-output", &[]),
                    e,
                )
            })? {
                LogOutput::StdOut { message } => stdout.extend_from_slice(&message),
                other => stderr.extend_from_slice(other.as_ref()),
            }
        }
        Ok(Ok(OneShot {
            code,
            stdout,
            stderr,
        }))
    }

    /// Docker 的网桥：按 Docker 的网络列表认，不按网卡名字的样子猜。
    pub(in crate::plugin) async fn bridges(&self) -> Result<DockerBridges, EngineError> {
        let langs = self.owner_languages();
        let networks = self.docker.list_networks(None).await.map_err(|e| {
            crate::plugin::docker::from_bollard(
                &langs,
                &crate::l10n::text(&langs, "plugin-settings-lan-detect-list-networks", &[]),
                e,
            )
        })?;
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
    pub async fn create_lan_network(
        &self,
        name: &str,
        labels: &HashMap<String, String>,
    ) -> Result<(), EngineError> {
        self.create_reserved(Self::lan_request(name, labels)).await
    }

    /// 同 [`Self::create_lan_network`]，但在迁移预留的网段上建。
    pub async fn create_lan_network_at(
        &self,
        name: &str,
        labels: &HashMap<String, String>,
        subnet: Ipv4Net,
    ) -> Result<(), EngineError> {
        self.create_network_at_reserved(Self::lan_request(name, labels), subnet)
            .await
    }

    fn lan_request(name: &str, labels: &HashMap<String, String>) -> NetworkCreateRequest {
        NetworkCreateRequest {
            name: name.to_owned(),
            driver: Some("bridge".to_owned()),
            labels: Some(labels.clone()),
            internal: Some(false),
            options: Some(HashMap::from([(MASQUERADE.to_owned(), "false".to_owned())])),
            ..Default::default()
        }
    }
}

/// Docker 连不上或没按时处理完照常报错；Docker 拒绝了（镜像没了、宿主不让用宿主网络等）带原错误回来，
/// 供调用方按原话分类。
fn unless_unavailable(e: EngineError) -> Result<EngineError, EngineError> {
    match e {
        EngineError::Unavailable(_) | EngineError::TimedOut(_) => Err(e),
        other => Ok(other),
    }
}
