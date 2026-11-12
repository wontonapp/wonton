//! 宿主网络的网页和 MCP 服务经 unix 套接字接进核心（ADR-0036）。
//!
//! 宿主开着防火墙时，桥接网络里的核心连不上宿主网络里的服务。中继容器和服务一样在宿主网络里，监听插件数据目录里的
//! unix 套接字，转发到 `127.0.0.1:<端口>`；unix 套接字不走防火墙，核心直接连文件。

use std::collections::BTreeMap;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::path::{Path, PathBuf};
use std::time::Duration;

use bollard::models::{ContainerCreateBody, HostConfig, RestartPolicy, RestartPolicyNameEnum};
use serde::{Deserialize, Serialize};

use super::compose::{NetworkMode, PullPolicy, Service};
use super::docker::Engine;
use super::hostdir::Owner;
use super::languages::Languages;
use super::manager::InstallError;
use super::manifest::Manifest;
use super::names::Instance;
use super::render::{ContainerPlan, LABEL_SERVICE};
use super::store::InstalledRecord;
use crate::l10n;

/// `alpine/socat`，ADR-0036 的验证结果用它实测过。GPL-2.0，作为独立容器运行。摘要是多架构 manifest
/// list 的（registry API 取，`mediaType` 是 manifest list，覆盖 amd64、arm64 等），Docker 按本机架构挑。
pub const IMAGE: &str =
    "alpine/socat:1.8.0.3@sha256:beb4a68d9e4fe6b0f21ea774a0fde6c31f580dde6368939ed70100c5385b015e";

/// AF_UNIX 套接字路径的上限：Linux 的 `sun_path` 有 108 字节，含结尾 NUL，路径本身最多 107 字节。
pub const MAX_SOCKET_PATH: usize = 107;

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

/// 核心一侧第一个放不下的套接字：超长时给人看的说明（路径、字节数、上限），
/// 用来在装之前报明白；都放得下时为空。
pub fn path_too_long(app_data: &Path, relays: &[HostRelay], langs: &Languages) -> Option<String> {
    relays
        .iter()
        .map(|relay| socket_path(app_data, relay))
        .find(|path| path.as_os_str().len() > MAX_SOCKET_PATH)
        .map(|path| {
            l10n::text(
                langs,
                "plugin-install-relay-path-too-long",
                &[
                    ("path", path.display().to_string().into()),
                    ("bytes", path.as_os_str().len().into()),
                    ("max", MAX_SOCKET_PATH.into()),
                ],
            )
        })
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
        bot: None,
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
        store_dir: None,
        one_shot: false,
        variables: Default::default(),
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
        let Some(relay) = relays.iter().find(|r| r.purpose == Purpose::Webui) else {
            return false;
        };
        if self.direct_webui_probe(manifest, network).await != Some(true) {
            return false;
        }
        let Some(webui) = &manifest.webui else {
            return false;
        };
        let name = container(&self.cfg.instance, &manifest.id.slug(), relay);
        listens_beyond_loopback(&self.engine, &name, webui.port, &self.owner_languages())
            .await
            .unwrap_or(false)
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
        let mirror = self.registry_mirror_address(&self.owner_languages())?;
        let image = self
            .engine
            .ensure_image(
                IMAGE,
                super::compose::PullPolicy::Missing,
                mirror.as_deref(),
            )
            .await?;
        let owner = self.owner();
        let app_data = self.cfg.apps_dir.join(manifest.id.slug());
        super::hostdir::ensure_owned_dir(&dir(&app_data), owner).map_err(|e| {
            let langs = self.owner_languages();
            InstallError::Storage(l10n::text(
                &langs,
                "plugin-install-relay-prepare-dir-failed",
                &[("error", e.to_string().into())],
            ))
        })?;
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
            let mut plan = plan(&self.cfg.instance, manifest, &app_data, owner, relay);
            // 摘要按渲染时的镜像算，所以要在换成拉取后的写法之前打。
            super::render_drift::stamp(&mut plan);
            plan.body.image = Some(image.clone());
            plan.image.clone_from(&image);
            self.engine.create_container(&name, plan.body).await?;
            self.engine.start_container(&name).await?;
        }
        Ok(())
    }
}

/// 网页端口有没有监听在回环之外。中继和应用在同一个网络命名空间里，读中继看到的监听表；
/// 读不到（执行失败、没有这个端口的监听）时返回 `None`，不把不确定说成防火墙。
pub(super) async fn listens_beyond_loopback(
    engine: &Engine,
    container: &str,
    port: u16,
    langs: &Languages,
) -> Option<bool> {
    let argv = ["cat", "/proc/net/tcp", "/proc/net/tcp6"].map(str::to_owned);
    let output = match engine
        .exec_output(container, &argv, None, Duration::from_secs(3))
        .await
    {
        Ok(output) => output,
        Err(error) => {
            tracing::warn!(
                %container,
                %error,
                "{}",
                l10n::text(langs, "plugin-install-relay-listens-unknown", &[])
            );
            return None;
        }
    };
    if output.exit_code != 0 {
        tracing::warn!(
            %container,
            stderr = output.stderr.trim(),
            "{}",
            l10n::text(langs, "plugin-install-relay-listens-unknown", &[])
        );
        return None;
    }
    let mut listening = false;
    for line in output.stdout.lines().skip(1) {
        let fields: Vec<&str> = line.split_whitespace().collect();
        let (Some(local), Some(state)) = (fields.get(1), fields.get(3)) else {
            continue;
        };
        if *state != "0A" {
            continue;
        }
        let Some((ip, port_hex)) = local.rsplit_once(':') else {
            continue;
        };
        if u16::from_str_radix(port_hex, 16).ok() != Some(port) {
            continue;
        }
        let Some(ip) = parse_proc_ip(ip) else {
            continue;
        };
        listening = true;
        if !ip.is_loopback() {
            return Some(true);
        }
    }
    if !listening {
        tracing::warn!(%container, port, "监听表里没有这个端口，不判断防火墙");
    }
    listening.then_some(false)
}

/// `/proc/net/tcp{,6}` 的地址：每个 32 位字按主机字节序写，IPv4 是一个字，IPv6 是四个。
pub(super) fn parse_proc_ip(hex: &str) -> Option<IpAddr> {
    fn word(chunk: &str) -> Option<[u8; 4]> {
        let value = u32::from_str_radix(chunk, 16).ok()?;
        Some(value.swap_bytes().to_be_bytes())
    }
    match hex.len() {
        8 => Some(IpAddr::V4(Ipv4Addr::from(word(hex)?))),
        32 => {
            let mut bytes = [0; 16];
            for (out, chunk) in bytes
                .as_chunks_mut::<4>()
                .0
                .iter_mut()
                .zip(hex.as_bytes().chunks(8))
            {
                out.copy_from_slice(&word(std::str::from_utf8(chunk).ok()?)?);
            }
            let ip = Ipv6Addr::from(bytes);
            Some(ip.to_ipv4_mapped().map_or(IpAddr::V6(ip), IpAddr::V4))
        }
        _ => None,
    }
}
