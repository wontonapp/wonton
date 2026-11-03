//! 把核心升级前建的插件网络迁移成留出核心地址的建法（ADR-0023 的“只信任核心的地址”）：主人不用操作，容器也不重建，
//! Bot 插件不换设备。
//!
//! 一次一个网络：把网段、选项、标签和每个容器的地址、别名、网关优先级存进核心的数据目录，断开所有容器，删掉网络，用同一个
//! 网段按新建法重建，再把容器接回去，地址和别名不变。实测每个网络不到 1 秒，地址、服务名、发布的端口和默认路由都保持，
//! 经这个网络的长连接没有断。迁移中途核心退出，下次启动从这份记录接着做。

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::PathBuf;

use bollard::models::{NetworkCreateRequest, NetworkInspect};
use bollard::query_parameters::{ListContainersOptionsBuilder, StopContainerOptionsBuilder};
use ipnet::Ipv4Net;
use serde::{Deserialize, Serialize};

use super::core_address::{Endpoint, last_usable, layout_of, reserved_ipam, subnet_taken};
use super::docker::{Engine, EngineError};
use super::manager::{Deployed, InstallError, PluginManager};
use super::manifest::Manifest;
use super::package::PluginPackage;
use super::store::{
    InstalledRecord, StoreError, create_private_dir, read_json, remove_file, write_json,
};

/// 迁移之前网络的样子，每个网络一个 JSON 文件。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Saved {
    subnet: Ipv4Net,
    gateway: Option<String>,
    driver: Option<String>,
    internal: bool,
    options: HashMap<String, String>,
    labels: HashMap<String, String>,
    /// 容器名 → 它在网络里的端点。核心不在里面：它接到留给它的地址上。
    endpoints: BTreeMap<String, Endpoint>,
    /// 迁移时停下的容器（在重启循环里的），接回去后要启动。
    #[serde(default)]
    stopped: BTreeSet<String>,
}

/// 进行中的迁移的记录。
pub struct MigrationStore {
    dir: PathBuf,
}

impl MigrationStore {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    fn path(&self, network: &str) -> PathBuf {
        self.dir.join(format!("{network}.json"))
    }
}

#[derive(Debug, thiserror::Error)]
pub enum MigrationError {
    #[error(transparent)]
    Engine(#[from] EngineError),
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error("网络 {0} 没有能给核心留地址的 IPv4 网段")]
    NoLayout(String),
}

impl From<MigrationError> for InstallError {
    fn from(e: MigrationError) -> Self {
        match e {
            MigrationError::Engine(engine) => engine.into(),
            MigrationError::Store(store) => store.into(),
            other @ MigrationError::NoLayout(_) => Self::Docker(other.to_string()),
        }
    }
}

impl Engine {
    /// 网络还没给核心留地址时迁移它，上次迁移没做完时接着做，返回做没做。`core` 是核心容器：它也要断开，但不由这里接回去。
    /// 网络不存在、也没有没做完的迁移时什么都不做，由核对插件时报错。
    pub async fn migrate_network(
        &self,
        network: &str,
        core: Option<&str>,
        store: &MigrationStore,
    ) -> Result<bool, MigrationError> {
        let found = self.inspect_network(network).await?;
        let mut saved = match read_json::<Saved>(&store.path(network))? {
            Some(saved) => saved,
            None => {
                let Some(found) = &found else {
                    return Ok(false);
                };
                let layout =
                    layout_of(found).ok_or_else(|| MigrationError::NoLayout(network.to_owned()))?;
                if layout.reserved {
                    return Ok(false);
                }
                let saved = self.read_network(found, core).await?;
                create_private_dir(&store.dir)?;
                write_json(&store.path(network), &saved)?;
                saved
            }
        };
        let reserved = found
            .as_ref()
            .and_then(layout_of)
            .is_some_and(|l| l.reserved);
        if !reserved {
            if found.is_some() {
                for name in self.attached_containers(network).await? {
                    self.detach(network, &name, &mut saved, store).await?;
                }
                self.remove_network(network).await?;
            }
            self.recreate(network, &saved).await?;
        }
        self.follow_new_subnet(network, &mut saved, store).await?;
        self.reconnect(network, &saved).await?;
        remove_file(&store.path(network))?;
        tracing::info!(%network, "插件网络已迁移，给核心留出了地址");
        Ok(true)
    }

    /// 重启循环中的容器没有端点，断开时 Docker 报“没接着”，配置里却还记着旧网络的 ID：网络重建后它再也起不来（Engine 29.8.2
    /// 实测）。要先停下才断得开。先记下再停：中途退出时，下次还知道要启动它。
    async fn detach(
        &self,
        network: &str,
        name: &str,
        saved: &mut Saved,
        store: &MigrationStore,
    ) -> Result<(), MigrationError> {
        self.disconnect_network(network, name).await?;
        if self.endpoint(name, network).await?.is_some() {
            saved.stopped.insert(name.to_owned());
            write_json(&store.path(network), saved)?;
            let stop = StopContainerOptionsBuilder::default().t(10).build();
            self.docker
                .stop_container(name, Some(stop))
                .await
                .map_err(|e| EngineError::from_bollard(&format!("停止容器 {name}"), e))?;
            self.disconnect_network(network, name).await?;
        }
        Ok(())
    }

    /// 接在网络上的容器，包括停着的：网络的查询接口只列运行中的，而停着的容器记着旧网络的 ID，不断开的话网络重建后它起不来。
    async fn attached_containers(&self, network: &str) -> Result<Vec<String>, EngineError> {
        let filters = HashMap::from([("network".to_owned(), vec![network.to_owned()])]);
        let containers = self
            .docker
            .list_containers(Some(
                ListContainersOptionsBuilder::default()
                    .all(true)
                    .filters(&filters)
                    .build(),
            ))
            .await
            .map_err(|e| EngineError::from_bollard(&format!("列出网络 {network} 上的容器"), e))?;
        Ok(containers
            .into_iter()
            .filter_map(|c| {
                c.names?
                    .first()
                    .map(|n| n.trim_start_matches('/').to_owned())
            })
            .collect())
    }

    async fn read_network(
        &self,
        found: &NetworkInspect,
        core: Option<&str>,
    ) -> Result<Saved, MigrationError> {
        let network = found.name.clone().unwrap_or_default();
        let no_layout = || MigrationError::NoLayout(network.clone());
        let layout = layout_of(found).ok_or_else(no_layout)?;
        // 放不下核心的地址就不动这个网络，免得断开容器、删掉网络后才发现建不回去。
        last_usable(layout.subnet).ok_or_else(no_layout)?;
        let mut endpoints = BTreeMap::new();
        for name in self.attached_containers(&network).await? {
            if Some(name.as_str()) == core {
                continue;
            }
            let settings = self.endpoint(&name, &network).await?.unwrap_or_default();
            let endpoint = Endpoint {
                address: settings.ip_address.filter(|ip| !ip.is_empty()),
                aliases: settings.aliases.unwrap_or_default(),
                gw_priority: settings.gw_priority,
            };
            endpoints.insert(name, endpoint);
        }
        Ok(Saved {
            subnet: layout.subnet,
            gateway: layout.gateway,
            driver: found.driver.clone(),
            internal: found.internal.unwrap_or(false),
            options: found.options.clone().unwrap_or_default(),
            labels: found.labels.clone().unwrap_or_default(),
            endpoints,
            stopped: BTreeSet::new(),
        })
    }

    async fn recreate(&self, network: &str, saved: &Saved) -> Result<(), MigrationError> {
        let ipam = reserved_ipam(saved.subnet, saved.gateway.clone())
            .ok_or_else(|| MigrationError::NoLayout(network.to_owned()))?;
        let fresh = NetworkCreateRequest {
            name: network.to_owned(),
            driver: saved.driver.clone(),
            labels: Some(saved.labels.clone()),
            internal: Some(saved.internal),
            options: Some(saved.options.clone()),
            ..Default::default()
        };
        let same = NetworkCreateRequest {
            ipam: Some(ipam),
            ..fresh.clone()
        };
        match self.create_raw(same).await {
            // 删网络的这一下，别的网络拿走了这个网段，之后也不会还回来。死守的话这个插件再也起不来，只好换一段。
            Err(e) if subnet_taken(&e) => {
                tracing::warn!(%network, subnet = %saved.subnet, error = %e, "网段被占了，换一段");
                Ok(self.create_reserved(fresh).await?)
            }
            other => Ok(other?),
        }
    }

    /// 网段换了时，记着的地址都作废，容器由 Docker 重新分配。
    async fn follow_new_subnet(
        &self,
        network: &str,
        saved: &mut Saved,
        store: &MigrationStore,
    ) -> Result<(), MigrationError> {
        let found = self.inspect_network(network).await?;
        let layout = found
            .as_ref()
            .and_then(layout_of)
            .ok_or_else(|| MigrationError::NoLayout(network.to_owned()))?;
        if layout.subnet != saved.subnet {
            saved.subnet = layout.subnet;
            saved.gateway = layout.gateway;
            for endpoint in saved.endpoints.values_mut() {
                endpoint.address = None;
            }
            write_json(&store.path(network), saved)?;
        }
        Ok(())
    }

    /// 地址和别名不变。正好占着留给核心的地址的容器由 Docker 重新分配。已经接回去的（上次做到一半）和已经不在的跳过。
    async fn reconnect(&self, network: &str, saved: &Saved) -> Result<(), MigrationError> {
        let core = last_usable(saved.subnet).map(|ip| ip.to_string());
        for (name, endpoint) in &saved.endpoints {
            if self.inspect_container(name).await?.is_none()
                || self.endpoint(name, network).await?.is_some()
            {
                continue;
            }
            let endpoint = Endpoint {
                address: endpoint
                    .address
                    .clone()
                    .filter(|a| Some(a) != core.as_ref()),
                ..endpoint.clone()
            };
            self.connect_network_at(network, name, endpoint).await?;
        }
        for name in &saved.stopped {
            if self.inspect_container(name).await?.is_some() {
                self.start_container(name).await?;
            }
        }
        Ok(())
    }
}

impl PluginManager {
    /// 迁移失败时不核对：网络可能还没建回去，把迁移的原因报给主人。
    pub(super) async fn reconcile_migrated(
        &self,
        migrated: Result<(), InstallError>,
        package: &PluginPackage,
        record: &InstalledRecord,
    ) -> Result<Deployed, InstallError> {
        migrated?;
        self.reconcile(package, record).await
    }

    /// 核心启动时迁移一个插件的网络，并把核心接回去：接下来的核对可能在别处就失败了，插件不健康也要连得上。
    pub(super) async fn migrate_network(
        &self,
        network: &str,
        manifest: &Manifest,
    ) -> Result<(), InstallError> {
        let core = self.cfg.core_container.as_deref();
        let store = self.store.network_migrations();
        let migrated = self.engine.migrate_network(network, core, store).await?;
        if let (true, Some(core)) = (migrated, core) {
            self.attach_core(network, core, manifest).await?;
        }
        Ok(())
    }
}
