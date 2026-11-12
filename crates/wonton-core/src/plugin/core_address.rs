//! 每个插件网络的最后一个可用地址留给核心（ADR-0023 的“只信任核心的地址”）。用请求头认用户的应用，信任的代理地址
//! 只写这个地址（`WONTON_CORE_ADDRESS`）：地址要固定，核心容器重建后还是它，不能为此重建插件。
//!
//! Engine 28 不许在自动分配的网段上指定地址，`AuxiliaryAddresses` 预留的地址谁都分配不了（都实测）。
//! 核心从自己的地址池挑 /26，显式 IPAM 的 `IPRange` 只给前一半，容器自动分配占不到后一半。

use std::collections::HashMap;
use std::net::Ipv4Addr;
use std::sync::Arc;
use std::time::Duration;

use bollard::models::{
    EndpointIpamConfig, EndpointSettings, Ipam, IpamConfig, NetworkConnectRequest,
    NetworkCreateRequest, NetworkInspect,
};
use ipnet::Ipv4Net;

use super::docker::{Engine, EngineError, GATEWAY_MODES, NetworkReach, from_bollard};
use super::languages::Languages;
use super::manager::{InstallError, PluginManager};
use super::manifest::{Manifest, Sso};
use crate::l10n;

/// 别的核心或程序可能同时选中同一段。沿用 #474 的重试次数与间隔，不把并发碰撞当地址池耗尽。
const CREATE_ATTEMPTS: u32 = 10;

/// 重试前等 50 到 500 毫秒。一起撞上的调用如果马上重来，又会一起选中同一段。
fn retry_delay() -> Duration {
    let mut bytes = [0; 2];
    getrandom::fill(&mut bytes).expect("操作系统随机数源不可用");
    Duration::from_millis(50 + u64::from(u16::from_le_bytes(bytes)) % 451)
}

/// 第一次渲染时网络还没建，用这个占位（文档用的地址，RFC 5737），建好网络后再渲染一次。`plugin check` 也用它判断
/// 插件有没有引用 `WONTON_CORE_ADDRESS`。
pub const PLACEHOLDER: &str = "192.0.2.254";

/// 注入给 `webui.sso: proxy` 的插件的插值变量。
pub const VAR: &str = "WONTON_CORE_ADDRESS";

/// 同一个地址的 CIDR 写法（`<地址>/32`），直接写进应用的“信任的代理地址”：有的应用只收 CIDR。
pub const CIDR_VAR: &str = "WONTON_GATEWAY_CIDR";

/// 留给核心的地址：网段的最后一个可用地址。
pub fn last_usable(subnet: Ipv4Net) -> Option<Ipv4Addr> {
    // 网段小到前一半和后一半放不下容器和核心时，Docker 也不会自动分配。
    (subnet.prefix_len() <= 30).then(|| Ipv4Addr::from(subnet.broadcast().to_bits() - 1))
}

/// 出口容器在插件网络里的固定地址：核心地址的前一个（倒数第二个可用地址）。网段小到只剩网关和核心时为空。
pub(super) fn egress_address(subnet: Ipv4Net, gateway: Option<&str>) -> Option<Ipv4Addr> {
    let address = Ipv4Addr::from(last_usable(subnet)?.to_bits() - 1);
    (address > subnet.network() && gateway != Some(address.to_string().as_str())).then_some(address)
}

/// 容器自动分配用的范围：网段的前一半。
fn lower_half(subnet: Ipv4Net) -> Option<Ipv4Net> {
    subnet.subnets(subnet.prefix_len() + 1).ok()?.next()
}

/// 插件网络的地址配置。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Layout {
    pub subnet: Ipv4Net,
    /// `isolated` 的网络 Docker 不分配网关。
    pub gateway: Option<String>,
    /// 配了 `IPRange`：已经给核心留出了地址。
    pub reserved: bool,
}

/// 第一个 IPv4 网段的配置。
pub(super) fn layout_of(network: &NetworkInspect) -> Option<Layout> {
    let configs = network.ipam.as_ref()?.config.as_ref()?;
    configs.iter().find_map(|config| {
        Some(Layout {
            subnet: config.subnet.as_deref()?.parse().ok()?,
            gateway: config.gateway.clone().filter(|g| !g.is_empty()),
            reserved: config.ip_range.as_deref().is_some_and(|r| !r.is_empty()),
        })
    })
}

/// 用 `subnet` 建网络时的 IPAM：`IPRange` 只给前一半。
pub(super) fn reserved_ipam(subnet: Ipv4Net, gateway: Option<String>) -> Option<Ipam> {
    last_usable(subnet)?;
    Some(Ipam {
        config: Some(vec![IpamConfig {
            subnet: Some(subnet.to_string()),
            ip_range: Some(lower_half(subnet)?.to_string()),
            gateway,
            ..Default::default()
        }]),
        ..Default::default()
    })
}

fn no_layout(langs: &Languages, context: &str) -> EngineError {
    EngineError::Rejected {
        context: context.to_owned(),
        message: l10n::text(langs, "plugin-install-coreaddr-no-ipv4-layout", &[]),
    }
}

/// 别的网络先拿走了这个网段。Docker 正在建那个网络时，报的是分配不了网关（实测）。
pub(super) fn subnet_taken(error: &EngineError) -> bool {
    matches!(error, EngineError::Rejected { message, .. }
        if message.contains("overlap") || message.contains("already in use"))
}

fn pool_exhausted_error(
    langs: &Languages,
    name: &str,
    pool: super::network_pool::NetworkPool,
) -> EngineError {
    let count = 1u32 << (26 - pool.subnet().prefix_len());
    EngineError::Rejected {
        context: l10n::text(
            langs,
            "plugin-install-coreaddr-pool-exhausted-context",
            &[
                ("name", name.into()),
                ("var", super::network_pool::VAR.into()),
            ],
        ),
        message: l10n::text(
            langs,
            "plugin-install-coreaddr-pool-exhausted",
            &[
                ("subnet", pool.subnet().to_string().into()),
                ("count", count.into()),
            ],
        ),
    }
}

/// 试满次数还是撞：仍是 Docker 的拒绝，原话不动，前面写明重试过。
fn kept_taken(langs: &Languages, error: EngineError) -> EngineError {
    match error {
        EngineError::Rejected { context, message } => EngineError::Rejected {
            context: l10n::text(
                langs,
                "plugin-install-coreaddr-kept-taken",
                &[
                    ("context", context.into()),
                    ("attempts", CREATE_ATTEMPTS.into()),
                ],
            ),
            message,
        },
        other => other,
    }
}

/// 渲染前预留的网段：同进程其他建网络会跳过它们。丢弃 [`Reservation`] 就放掉（渲染失败时旧
/// 网络和容器一个没动）；网络建好后也丢弃（Docker 的网络列表里已经看得见）。
#[derive(Debug)]
pub(in crate::plugin) struct Reservation {
    /// 还没发给调用方的网段。
    pending: Vec<Ipv4Net>,
    /// 预留的全量：Drop 按它放掉，`next` 取走的不影响释放。
    reserved: Vec<Ipv4Net>,
    held: Arc<std::sync::Mutex<Vec<Ipv4Net>>>,
}

impl Reservation {
    /// 按预留顺序取网段：第一个给主网络，往后依次是局域网、出口。
    pub(in crate::plugin) fn next(&mut self) -> Option<Ipv4Net> {
        if self.pending.is_empty() {
            None
        } else {
            Some(self.pending.remove(0))
        }
    }
}

impl Drop for Reservation {
    fn drop(&mut self) {
        let mut held = self.held.lock().unwrap_or_else(|e| e.into_inner());
        held.retain(|subnet| !self.reserved.contains(subnet));
    }
}

impl Engine {
    /// 建插件网络，给核心留出最后一个可用地址。
    pub async fn create_network(
        &self,
        name: &str,
        labels: &HashMap<String, String>,
        reach: NetworkReach,
    ) -> Result<(), EngineError> {
        self.create_reserved(Self::reserved_request(name, labels, reach))
            .await
    }

    /// 同 [`Self::create_network`]，但在迁移预留的网段上建。
    pub(super) async fn create_network_at(
        &self,
        name: &str,
        labels: &HashMap<String, String>,
        reach: NetworkReach,
        subnet: Ipv4Net,
    ) -> Result<(), EngineError> {
        self.create_network_at_reserved(Self::reserved_request(name, labels, reach), subnet)
            .await
    }

    fn reserved_request(
        name: &str,
        labels: &HashMap<String, String>,
        reach: NetworkReach,
    ) -> NetworkCreateRequest {
        let isolated = GATEWAY_MODES.map(|mode| (mode.to_owned(), "isolated".to_owned()));
        NetworkCreateRequest {
            name: name.to_owned(),
            driver: Some("bridge".to_owned()),
            labels: Some(labels.clone()),
            internal: Some(reach != NetworkReach::Egress),
            options: (reach == NetworkReach::Isolated).then(|| isolated.into()),
            ..Default::default()
        }
    }

    /// 宿主路由只查一次（一次建网络的过程里不会变），Docker 网段每轮重查；Docker 报重叠时换下一段，
    /// 保留这轮已撞的网段。整个挑选加创建在 [`Engine::reserved_creation`] 锁内进行，本进程的
    /// 并发建网络排队挨个挑段；锁外的并发（外部程序）才靠重试。
    pub(super) async fn create_reserved(
        &self,
        request: NetworkCreateRequest,
    ) -> Result<(), EngineError> {
        let _serial = self.reserved_creation.lock().await;
        let host = self.host_subnets(&request.name).await?;
        let mut taken = Vec::new();
        for attempt in 1..=CREATE_ATTEMPTS {
            let mut used = self.docker_subnets(&request.name).await?;
            used.extend(host.iter().copied());
            used.extend_from_slice(&taken);
            used.extend(
                self.held_subnets
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .iter()
                    .copied(),
            );
            let subnet = self.network_pool.available(&used).ok_or_else(|| {
                pool_exhausted_error(&self.owner_languages(), &request.name, self.network_pool)
            })?;
            let isolated = request
                .options
                .as_ref()
                .and_then(|options| options.get(GATEWAY_MODES[0]))
                .is_some_and(|mode| mode == "isolated");
            let gateway =
                (!isolated).then(|| Ipv4Addr::from(subnet.network().to_bits() + 1).to_string());
            let ipam = reserved_ipam(subnet, gateway).expect("/26 能给核心留地址");
            match self
                .create_raw(NetworkCreateRequest {
                    ipam: Some(ipam),
                    ..request.clone()
                })
                .await
            {
                Err(error) if subnet_taken(&error) && attempt < CREATE_ATTEMPTS => {
                    taken.push(subnet);
                    tokio::time::sleep(retry_delay()).await;
                }
                Err(error) if subnet_taken(&error) => {
                    return Err(kept_taken(&self.owner_languages(), error));
                }
                other => return other,
            }
        }
        unreachable!("最后一次创建的结果直接返回")
    }

    /// 网络的网段是否不在这套核心的地址段里。旧版核心让 Docker 从默认地址池分插件网络的网段，主人升级
    /// 核心后，下一次为这个插件重建容器时把它删掉在段内重建（卷和数据不动）；网络不存在时不迁，新建走
    /// [`Self::create_reserved`]。没写 IPAM 网段的网络不归这套判断管，不迁。
    pub(super) async fn network_outside_pool(&self, name: &str) -> Result<bool, EngineError> {
        let Some(found) = self.inspect_network(name).await? else {
            return Ok(false);
        };
        let subnets: Vec<Ipv4Net> = found
            .ipam
            .and_then(|ipam| ipam.config)
            .unwrap_or_default()
            .iter()
            .filter_map(|config| config.subnet.as_deref().and_then(|text| text.parse().ok()))
            .collect();
        Ok(!subnets.is_empty() && !subnets.iter().all(|s| self.network_pool.covers(*s)))
    }

    /// 给 `count` 个网络各挑一个空闲网段并预留：同进程并发的 [`Self::create_reserved`] 会跳过
    /// 这些段，外部程序同时建的仍可能在创建时撞上（那时按失败即停报错）。渲染失败丢弃返回的
    /// [`Reservation`] 就放掉了，什么都没动。
    pub(in crate::plugin) async fn reserve_subnets(
        &self,
        count: usize,
        name: &str,
    ) -> Result<Reservation, EngineError> {
        let _serial = self.reserved_creation.lock().await;
        let mut used = self.docker_subnets(name).await?;
        used.extend(self.host_subnets(name).await?.iter().copied());
        used.extend(
            self.held_subnets
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .iter()
                .copied(),
        );
        let langs = self.owner_languages();
        let mut pending = Vec::with_capacity(count);
        for _ in 0..count {
            let subnet = self
                .network_pool
                .available(&used)
                .ok_or_else(|| pool_exhausted_error(&langs, name, self.network_pool))?;
            pending.push(subnet);
            used.push(subnet);
        }
        self.held_subnets
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .extend(pending.iter().copied());
        let reserved = pending.clone();
        Ok(Reservation {
            pending,
            reserved,
            held: Arc::clone(&self.held_subnets),
        })
    }

    /// 在预留的网段上建网络。别的程序刚好同时占了这一段时按失败即停报错：变量已经按这段算了，
    /// 不能悄悄换段（换段要重新渲染）。调用方负责丢弃预留。
    pub(super) async fn create_network_at_reserved(
        &self,
        request: NetworkCreateRequest,
        subnet: Ipv4Net,
    ) -> Result<(), EngineError> {
        let isolated = request
            .options
            .as_ref()
            .and_then(|options| options.get(GATEWAY_MODES[0]))
            .is_some_and(|mode| mode == "isolated");
        let gateway =
            (!isolated).then(|| Ipv4Addr::from(subnet.network().to_bits() + 1).to_string());
        let langs = self.owner_languages();
        let ipam = reserved_ipam(subnet, gateway).expect("/26 能给核心留地址");
        let name = request.name.clone();
        match self
            .create_raw(NetworkCreateRequest {
                ipam: Some(ipam),
                ..request
            })
            .await
        {
            Ok(()) => Ok(()),
            Err(error) if subnet_taken(&error) => Err(EngineError::Rejected {
                context: l10n::text(
                    &langs,
                    "plugin-install-coreaddr-context-create-network",
                    &[("name", name.into())],
                ),
                message: l10n::text(
                    &langs,
                    "plugin-install-coreaddr-reserved-taken",
                    &[("subnet", subnet.to_string().into())],
                ),
            }),
            Err(error) => Err(error),
        }
    }

    pub(super) async fn create_raw(
        &self,
        request: NetworkCreateRequest,
    ) -> Result<(), EngineError> {
        let langs = self.owner_languages();
        let context = l10n::text(
            &langs,
            "plugin-install-coreaddr-context-create-network",
            &[("name", request.name.clone().into())],
        );
        self.docker
            .create_network(request)
            .await
            .map(|_| ())
            .map_err(|e| from_bollard(&langs, &context, e))
    }

    /// 查询网络；不存在时返回 `None`。
    pub(super) async fn inspect_network(
        &self,
        name: &str,
    ) -> Result<Option<NetworkInspect>, EngineError> {
        let langs = self.owner_languages();
        let context = l10n::text(
            &langs,
            "plugin-install-coreaddr-context-inspect-network",
            &[("name", name.into())],
        );
        match self.docker.inspect_network(name, None).await {
            Ok(network) => Ok(Some(network)),
            Err(e) => match from_bollard(&langs, &context, e) {
                EngineError::NotFound(_) => Ok(None),
                other => Err(other),
            },
        }
    }

    /// 插件网络里留给核心的地址。网络还没留出来（核心升级前建的）时报错：这时指定地址接入，Engine 28 会拒绝。
    pub async fn core_address(&self, network: &str) -> Result<Ipv4Addr, EngineError> {
        let langs = self.owner_languages();
        let context = l10n::text(
            &langs,
            "plugin-install-coreaddr-context-inspect-network",
            &[("name", network.into())],
        );
        let found = self
            .inspect_network(network)
            .await?
            .ok_or_else(|| EngineError::NotFound(context.clone()))?;
        let layout = layout_of(&found).ok_or_else(|| no_layout(&langs, &context))?;
        if !layout.reserved {
            return Err(EngineError::Rejected {
                context,
                message: l10n::text(&langs, "plugin-install-coreaddr-no-core-address", &[]),
            });
        }
        last_usable(layout.subnet).ok_or_else(|| no_layout(&langs, &context))
    }

    /// 容器在网络里的端点，容器停着也有；容器不在或者没接这个网络时为空。
    pub(super) async fn endpoint(
        &self,
        container: &str,
        network: &str,
    ) -> Result<Option<EndpointSettings>, EngineError> {
        let info = self.inspect_container(container).await?;
        Ok(info
            .and_then(|info| info.network_settings)
            .and_then(|settings| settings.networks)
            .and_then(|mut networks| networks.remove(network)))
    }

    /// 容器在网络里的 IPv4 地址；容器不在、没接这个网络或者停着时为空。
    pub async fn endpoint_address(
        &self,
        container: &str,
        network: &str,
    ) -> Result<Option<String>, EngineError> {
        let endpoint = self.endpoint(container, network).await?;
        Ok(endpoint
            .and_then(|endpoint| endpoint.ip_address)
            .filter(|ip| !ip.is_empty()))
    }

    /// 以指定的地址、别名和网关优先级接入网络。优先级的道理见 [`Engine::connect_network`]。
    pub async fn connect_network_at(
        &self,
        network: &str,
        container: &str,
        endpoint: Endpoint,
    ) -> Result<(), EngineError> {
        let request = NetworkConnectRequest {
            container: container.to_owned(),
            endpoint_config: Some(EndpointSettings {
                aliases: Some(endpoint.aliases),
                gw_priority: endpoint.gw_priority,
                ipam_config: endpoint.address.map(|address| EndpointIpamConfig {
                    ipv4_address: Some(address),
                    ..Default::default()
                }),
                ..Default::default()
            }),
        };
        self.docker
            .connect_network(network, request)
            .await
            .map_err(|e| {
                let langs = self.owner_languages();
                from_bollard(
                    &langs,
                    &l10n::text(
                        &langs,
                        "plugin-install-docker-context-connect-network",
                        &[("container", container.into()), ("network", network.into())],
                    ),
                    e,
                )
            })
    }
}

/// 容器在网络里的端点。
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Endpoint {
    /// 指定的 IPv4 地址；为空时由 Docker 分配。
    pub address: Option<String>,
    pub aliases: Vec<String>,
    pub gw_priority: Option<i64>,
}

impl PluginManager {
    /// 把核心接到网络里留给它的地址上；已经接着但地址不对时断开重接。
    pub(super) async fn attach_core(
        &self,
        network: &str,
        core: &str,
        manifest: &Manifest,
    ) -> Result<(), InstallError> {
        self.attach_mailpit(network, manifest).await?;
        let address = self.engine.core_address(network).await?.to_string();
        let current = self.engine.endpoint_address(core, network).await?;
        if current.as_deref() == Some(address.as_str()) {
            return Ok(());
        }
        if current.is_some() {
            self.engine.disconnect_network(network, core).await?;
        }
        let endpoint = Endpoint {
            address: Some(address),
            aliases: self.core_aliases_of(manifest),
            gw_priority: Some(-1),
        };
        self.engine
            .connect_network_at(network, core, endpoint)
            .await?;
        Ok(())
    }

    /// 声明了 `webui.sso: proxy` 时注入的 `WONTON_CORE_ADDRESS`。核心直接跑在宿主上（开发时）时是网桥的网关，这条
    /// 不起作用，只用于开发。
    pub(super) async fn core_address_var(
        &self,
        manifest: &Manifest,
        network: &str,
    ) -> Result<Option<String>, InstallError> {
        if manifest.webui.as_ref().is_none_or(|w| w.sso != Sso::Proxy) {
            return Ok(None);
        }
        let address = match &self.cfg.core_container {
            Some(_) => self.engine.core_address(network).await?.to_string(),
            None => self.engine.network_gateway(network).await?,
        };
        Ok(Some(address))
    }
}

#[cfg(test)]
mod tests {
    use super::super::fake_docker::FakeDocker;
    use super::*;

    const OVERLAP: &str = "cannot create network a (br-a): conflicts with network b (br-b): \
                           networks have overlapping IPv4";

    #[tokio::test]
    async fn a_subnet_taken_again_and_again_is_retried_until_the_network_keeps_the_core_address() {
        let (docker, engine) = FakeDocker::start().await;
        docker.reject_pinned_networks(3, OVERLAP);
        engine
            .create_network("n", &HashMap::new(), NetworkReach::Isolated)
            .await
            .unwrap();
        engine.core_address("n").await.unwrap();
    }

    #[tokio::test]
    async fn a_subnet_that_stays_taken_is_reported_as_a_docker_rejection() {
        let (docker, engine) = FakeDocker::start().await;
        docker.reject_pinned_networks(CREATE_ATTEMPTS, OVERLAP);
        let error = engine
            .create_network("n", &HashMap::new(), NetworkReach::Isolated)
            .await
            .unwrap_err();
        assert!(subnet_taken(&error), "{error}");
        assert!(docker.networks().is_empty());
    }

    #[tokio::test]
    async fn only_a_taken_subnet_is_retried() {
        let (docker, engine) = FakeDocker::start().await;
        docker.reject_pinned_networks(1, "permission denied");
        let error = engine
            .create_network("n", &HashMap::new(), NetworkReach::Isolated)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("permission denied"), "{error}");
        assert!(docker.networks().is_empty());
        assert_eq!(docker.writes(), ["POST /networks/create"]);
    }

    fn net(text: &str) -> Ipv4Net {
        text.parse().unwrap()
    }

    #[test]
    fn the_core_gets_the_last_usable_address_and_containers_the_lower_half() {
        let cases = [
            ("172.18.0.0/16", "172.18.255.254", "172.18.0.0/17"),
            ("192.168.32.0/20", "192.168.47.254", "192.168.32.0/21"),
            ("10.1.2.0/24", "10.1.2.254", "10.1.2.0/25"),
            ("10.213.0.64/26", "10.213.0.126", "10.213.0.64/27"),
        ];
        for (subnet, core, lower) in cases {
            assert_eq!(last_usable(net(subnet)).unwrap().to_string(), core);
            assert_eq!(lower_half(net(subnet)).unwrap().to_string(), lower);
        }
        assert_eq!(last_usable(net("10.0.0.0/31")), None);
    }
}
