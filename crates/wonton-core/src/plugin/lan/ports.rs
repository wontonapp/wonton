//! 局域网端口被宿主上别的东西占着（#631）：Docker 里已发布的端口，和启动失败时 Docker 的原话。
//!
//! 装之前先跟 Docker 里所有运行中的容器对一遍，主人确认时就能看到谁占着；装到启动那步才发现的话，Docker 会 500，
//! 原话在这里认出来。宿主上非 Docker 的进程看不到，由预览时宿主网络里的探测补（[`unheard`]）。
//! 自己的容器不算冲突：升级、重建都是先删自己的容器再起新的。

use std::net::IpAddr;

use bollard::models::{ContainerSummaryStateEnum, PortSummaryTypeEnum};
use bollard::query_parameters::ListContainersOptionsBuilder;

use super::super::docker::{Engine, EngineError, from_bollard};
use super::{LanPortSpec, Protocol};
use crate::l10n;
use crate::plugin::languages::Languages;

/// 一个宿主端口被 Docker 里的容器发布着。宿主上非 Docker 的进程看不到。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublishedPort {
    /// 容器名。
    pub container: String,
    /// 容器属于哪个插件（`wonton.plugin` 标签）；别人的容器没有。
    pub plugin: Option<String>,
    /// 发布在哪个地址上，`0.0.0.0` 和 `::` 是通配。
    pub address: IpAddr,
    pub port: u16,
    pub protocol: Protocol,
}

impl Engine {
    /// 宿主上已被 Docker 里运行中的容器发布的端口。停着、刚建好的容器不占端口，不算；暂停、重启中的还算。
    pub(in crate::plugin) async fn published_ports(
        &self,
    ) -> Result<Vec<PublishedPort>, EngineError> {
        let containers = self
            .docker
            .list_containers(Some(
                ListContainersOptionsBuilder::default().all(true).build(),
            ))
            .await
            .map_err(|e| from_bollard(&self.owner_languages(), "列出容器已发布的端口", e))?;
        Ok(containers
            .into_iter()
            .filter(|c| {
                matches!(
                    c.state,
                    Some(
                        ContainerSummaryStateEnum::RUNNING
                            | ContainerSummaryStateEnum::PAUSED
                            | ContainerSummaryStateEnum::RESTARTING
                            | ContainerSummaryStateEnum::STOPPING
                    )
                )
            })
            .flat_map(|c| {
                let name = c
                    .names
                    .and_then(|n| n.into_iter().next())
                    .map(|n| n.trim_start_matches('/').to_owned())
                    .unwrap_or_default();
                let plugin = c
                    .labels
                    .as_ref()
                    .and_then(|l| l.get(super::super::names::LABEL_PLUGIN).cloned());
                c.ports
                    .unwrap_or_default()
                    .into_iter()
                    .filter_map(move |p| {
                        let address = p.ip.as_deref()?.parse().ok()?;
                        Some(PublishedPort {
                            container: name.clone(),
                            plugin: plugin.clone(),
                            address,
                            port: p.public_port?,
                            protocol: match p.typ {
                                Some(PortSummaryTypeEnum::UDP) => Protocol::Udp,
                                _ => Protocol::Tcp,
                            },
                        })
                    })
            })
            .collect())
    }
}

/// 一处冲突：要开的地址和端口，和占着它的容器。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Taken {
    /// 插件要开的地址。
    pub address: IpAddr,
    pub port: u16,
    pub protocol: Protocol,
    /// 占着它的容器；探测出来的宿主进程没有名字。
    pub container: Option<String>,
    /// 占着它的是别的插件时，那个插件的 ID。
    pub plugin: Option<String>,
}

impl Taken {
    /// 谁占着它，按语言写。
    pub fn who(&self, langs: &Languages) -> String {
        match (&self.plugin, &self.container) {
            (Some(id), _) => l10n::text(
                langs,
                "plugin-lan-taken-by-plugin",
                &[("id", id.as_str().into())],
            ),
            (None, Some(name)) => l10n::text(
                langs,
                "plugin-lan-taken-by-container",
                &[("name", name.as_str().into())],
            ),
            (None, None) => l10n::text(langs, "plugin-lan-taken-by-host", &[]),
        }
    }

    /// 给主人看的一句：哪个地址的哪个端口，被谁占着，装不装得上。`hint` 是只提示不挡的那种。
    pub fn describe(&self, langs: &Languages, hint: bool) -> String {
        let id = if hint {
            "plugin-install-lan-port-hinted"
        } else {
            "plugin-install-lan-port-taken"
        };
        l10n::text(
            langs,
            id,
            &[
                ("place", format!("{}:{}", self.address, self.port).into()),
                ("who", self.who(langs).into()),
            ],
        )
    }
}

/// 同一个端口和协议，地址相同或者任一边是通配（`0.0.0.0`、`::`）就算撞上。
fn same_place(published: &PublishedPort, address: IpAddr, port: u16, protocol: Protocol) -> bool {
    published.port == port
        && published.protocol == protocol
        && (published.address == address
            || published.address.is_unspecified()
            || address.is_unspecified())
}

/// `published` 里跟 `ports`（要开在 `addresses` 上的）撞上的。`own` 是这个插件的 ID，它自己的容器不算。
pub(in crate::plugin) fn clashes(
    published: &[PublishedPort],
    own: Option<&str>,
    addresses: &[IpAddr],
    ports: &[LanPortSpec],
) -> Vec<Taken> {
    let mut found = Vec::new();
    for port in ports {
        for &address in addresses {
            for published in published {
                if published.plugin.as_deref() == own
                    || !same_place(published, address, port.port, port.protocol)
                {
                    continue;
                }
                // 同一处只记一次。
                if found
                    .iter()
                    .any(|t: &Taken| t.address == address && t.port == port.port)
                {
                    continue;
                }
                found.push(Taken {
                    address,
                    port: port.port,
                    protocol: port.protocol,
                    container: Some(published.container.clone()),
                    plugin: published.plugin.clone(),
                });
            }
        }
    }
    found
}

/// 探测听见、Docker 列表里又没有的地址和端口：宿主上的程序占着。Docker 那边报过的就不再重复。
pub(in crate::plugin) fn unheard(
    heard: &[(IpAddr, u16)],
    published: &[PublishedPort],
) -> Vec<Taken> {
    // 连一下只判得了 TCP：探测的目标里本来只放 TCP 端口，这里也一律按 TCP 算。
    heard
        .iter()
        .filter(|(address, port)| {
            !published
                .iter()
                .any(|published| same_place(published, *address, *port, Protocol::Tcp))
        })
        .map(|&(address, port)| Taken {
            address,
            port,
            protocol: Protocol::Tcp,
            container: None,
            plugin: None,
        })
        .collect()
}

/// 从原话里取出 `地址:端口`：`Bind for 0.0.0.0:2222 failed: …`、`failed to bind host port 192.168.0.7:2222/tcp: …`、
/// `listen tcp4 0.0.0.0:2222: bind: …` 都取到第一个空格为止，再去掉 `/协议` 和结尾的冒号。
fn place(message: &str) -> Option<String> {
    [
        "Bind for ",
        "failed to bind host port ",
        "listen tcp4 ",
        "listen tcp6 ",
    ]
    .iter()
    .find_map(|marker| {
        let rest = message.split_once(marker)?.1;
        let token = rest.split_whitespace().next()?;
        let place = token.split('/').next()?.trim_end_matches(':');
        place.contains(':').then(|| place.to_owned())
    })
}

/// Docker 因为端口被占拒绝启动容器的原话 → 被占的 `地址:端口`（原话里取不到时 `Some(None)`）。不是端口的事时 `None`。
/// 只看两句特征话：分配器挡下的 `port is already allocated`，和真要绑时失败的 `address already in use`。前缀随版本变
/// （28 起多一层 `failed to set up container networking`，更早是 `driver failed programming external connectivity`，
/// 或者 `Error starting userland proxy`），所以不认前缀。句子由调用方按语言拼。
pub(in crate::plugin) fn taken(message: &str) -> Option<Option<String>> {
    let conflict =
        message.contains("port is already allocated") || message.contains("address already in use");
    conflict.then(|| place(message))
}
