//! 宿主网络的网页和 MCP 服务经 unix 套接字接进核心（ADR-0036）。
//!
//! 宿主开着防火墙时，桥接网络里的核心连不上宿主网络里的服务。中继容器和服务一样在宿主网络里，监听插件数据目录里的
//! unix 套接字，转发到 `127.0.0.1:<端口>`；unix 套接字不走防火墙，核心直接连文件。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use bollard::models::{ContainerCreateBody, HostConfig, RestartPolicy, RestartPolicyNameEnum};
use serde::{Deserialize, Serialize};

use super::compose::{NetworkMode, PullPolicy, Service};
use super::hostdir::Owner;
use super::manager::InstallError;
use super::manifest::Manifest;
use super::names::Instance;
use super::render::{ContainerPlan, LABEL_SERVICE};
use super::store::InstalledRecord;

/// `alpine/socat`，ADR-0036 的验证结果用它实测过。GPL-2.0，作为独立容器运行。
pub const IMAGE: &str = "alpine/socat:1.8.0.3";

/// 套接字接的是网页还是 MCP。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Purpose {
    Webui,
    Mcp,
}

impl Purpose {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Webui => "webui",
            Self::Mcp => "mcp",
        }
    }
}

/// 宿主网络里一个要接进核心的服务。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostRelay {
    pub purpose: Purpose,
    pub service: String,
    pub port: u16,
}

impl HostRelay {
    /// 套接字文件名：清单里 `webui`、`mcp` 各只有一个。
    fn socket(&self) -> String {
        format!("{}.sock", self.purpose.as_str())
    }

    /// 中继容器和 `wonton.service` 标签用的服务名。
    pub fn service_name(&self) -> String {
        format!("relay-{}", self.purpose.as_str())
    }
}

/// 容器在插件数据目录里的 `relay` 子目录。
pub fn dir(app_data: &Path) -> PathBuf {
    app_data.join("relay")
}

/// 套接字的绝对路径。
pub fn socket_path(app_data: &Path, relay: &HostRelay) -> PathBuf {
    dir(app_data).join(relay.socket())
}

/// 宿主网络的 `webui`、`mcp` 服务各要一个中继。
pub fn needed(manifest: &Manifest, services: &BTreeMap<String, Service>) -> Vec<HostRelay> {
    let declared = [
        (
            Purpose::Webui,
            manifest.webui.as_ref().map(|w| (&w.service, w.port)),
        ),
        (
            Purpose::Mcp,
            manifest.mcp.as_ref().map(|m| (&m.service, m.port)),
        ),
    ];
    declared
        .into_iter()
        .filter_map(|(purpose, spec)| {
            let (service, port) = spec?;
            let host = services
                .get(service)
                .is_some_and(|s| s.network_mode == Some(NetworkMode::Host));
            host.then(|| HostRelay {
                purpose,
                service: service.clone(),
                port,
            })
        })
        .collect()
}

/// 中继容器名。
pub fn container(instance: &Instance, slug: &str, relay: &HostRelay) -> String {
    instance.container(slug, &relay.service_name())
}

/// 中继容器：宿主网络里监听套接字，转发到服务的回环端口。
pub fn plan(
    instance: &Instance,
    manifest: &Manifest,
    app_data: &Path,
    owner: Owner,
    relay: &HostRelay,
) -> ContainerPlan {
    let service = relay.service_name();
    let name = container(instance, &manifest.id.slug(), relay);
    let mut labels = instance.labels(&manifest.id);
    labels.insert(LABEL_SERVICE.to_owned(), service.clone());
    let inside = format!("/s/{}", relay.socket());
    ContainerPlan {
        service: service.clone(),
        name: name.clone(),
        image: IMAGE.to_owned(),
        pull_policy: PullPolicy::Missing,
        depends_on: Vec::new(),
        body: ContainerCreateBody {
            image: Some(IMAGE.to_owned()),
            cmd: Some(vec![
                format!("UNIX-LISTEN:{inside},fork,mode=0660,unlink-early"),
                format!("TCP:127.0.0.1:{}", relay.port),
            ]),
            user: Some(format!("{}:{}", owner.uid, owner.gid)),
            labels: Some(labels),
            host_config: Some(HostConfig {
                network_mode: Some("host".to_owned()),
                binds: Some(vec![format!("{}:/s", dir(app_data).display())]),
                restart_policy: Some(RestartPolicy {
                    name: Some(RestartPolicyNameEnum::UNLESS_STOPPED),
                    ..Default::default()
                }),
                cap_drop: Some(vec!["ALL".to_owned()]),
                security_opt: Some(vec!["no-new-privileges".to_owned()]),
                readonly_rootfs: Some(true),
                ..Default::default()
            }),
            ..Default::default()
        },
        files: Vec::new(),
        one_shot: false,
    }
}

impl super::manager::PluginManager {
    /// 装完后的直连探测：只有宿主网络里真有网页时才试。
    pub(super) async fn host_webui_firewalled(
        &self,
        manifest: &Manifest,
        relays: &[HostRelay],
        network: &str,
    ) -> bool {
        if !relays.iter().any(|r| r.purpose == Purpose::Webui) {
            return false;
        }
        self.direct_webui_probe(manifest, network).await == Some(true)
    }

    /// 不经中继直连宿主网络里的网页：连不上时局域网设备可能也连不进来（本机防火墙）。
    pub(super) async fn direct_webui_probe(
        &self,
        manifest: &Manifest,
        network: &str,
    ) -> Option<bool> {
        let Some(webui) = &manifest.webui else {
            return None;
        };
        let host = match &self.cfg.core_container {
            Some(_) => match self.engine.network_gateway(network).await {
                Ok(gateway) => gateway,
                // 内部网络没有默认网关，网桥地址仍是网段的第一个地址。
                Err(error) => match self.engine.network_subnets(network).await {
                    Ok(subnets) => match subnets.first().and_then(|s| s.first_host()) {
                        Some(ip) => ip.to_string(),
                        None => {
                            tracing::warn!(plugin = %manifest.id, %error, "查不到网络的网桥地址，不做直连探测");
                            return None;
                        }
                    },
                    Err(error) => {
                        tracing::warn!(plugin = %manifest.id, %error, "查不到网络的网关，不做直连探测");
                        return None;
                    }
                },
            },
            None => "127.0.0.1".to_owned(),
        };
        let url = format!("http://{host}:{}{}", webui.port, webui.path);
        Some(
            self.http
                .get_status(&url, std::time::Duration::from_secs(3))
                .await
                .is_err(),
        )
    }

    /// 核心重启后中继可能不在了：按记录补上。
    pub(super) async fn ensure_relays(
        &self,
        manifest: &Manifest,
        record: &InstalledRecord,
    ) -> Result<(), InstallError> {
        if record.relays.is_empty() {
            return Ok(());
        }
        self.engine
            .ensure_image(IMAGE, super::compose::PullPolicy::Missing)
            .await?;
        let owner = self.owner();
        let app_data = self.cfg.apps_dir.join(manifest.id.slug());
        super::hostdir::ensure_owned_dir(&dir(&app_data), owner)
            .map_err(|e| InstallError::Storage(format!("准备中继目录失败：{e}")))?;
        for relay in &record.relays {
            let name = container(&self.cfg.instance, &manifest.id.slug(), relay);
            let running = self
                .engine
                .inspect_container(&name)
                .await?
                .and_then(|info| info.state)
                .and_then(|state| state.running)
                == Some(true);
            if running {
                continue;
            }
            self.engine.remove_container(&name).await?;
            let plan = plan(&self.cfg.instance, manifest, &app_data, owner, relay);
            self.engine.create_container(&name, plan.body).await?;
            self.engine.start_container(&name).await?;
        }
        Ok(())
    }
}
