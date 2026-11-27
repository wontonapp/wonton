//! 局域网转发（ADR-0021“局域网直连”）：一个 HAProxy TCP 容器，把发布在选中的局域网地址上的端口带 PROXY v2
//! 交给入口的局域网前端，不解 TLS，什么密钥都没有。
//!
//! 两张网络都由核心建：`lanpub` 是关掉地址伪装的桥接，端口发布在这里，转发不能经它出网；`lanfwd` 内部、隔离，
//! 上面只有转发和入口，连接器不在。网段从核心的地址池挑（避开宿主路由和别的 Docker 网络），转发和入口用网段末尾
//! 核心留的地址（自动分配只用前一半），容器重建也不变：入口按对端地址认转发，转发只听自己在 `lanpub` 上的地址。

use std::hash::{Hash, Hasher};
use std::net::{Ipv4Addr, SocketAddrV4};

use super::create::{Side, mounts};
use super::{EntryContainers, EntryError, LAN_FORWARD_COMPONENT};
use crate::entry_haproxy::{Forward, LAN_BIND, render_forward};
use crate::plugin::core_address::{Endpoint, Layout, last_usable, layout_of};
use crate::plugin::docker::NetworkReach;
use crate::plugin::names::{LABEL_COMPONENT, LABEL_MANAGED_BY};

/// 转发在容器里听的端口；宿主上发布的是 `WONTON_CLIENT_PORT`。
const LISTEN_PORT: u16 = 8483;
const CONFIG: &str = "lan-forward.cfg";
/// 配置文件的摘要：配置是挂进去的文件，内容变了（放行的网段换了）要重建，靠它认出来。
const CONFIG_LABEL: &str = "wonton.lan-forward.config";

/// `lanfwd` 和入口、转发在上面的地址。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LanLink {
    pub(crate) network: String,
    pub(crate) entry: Ipv4Addr,
    pub(crate) forward: Ipv4Addr,
}

/// `lanpub` 和转发在上面的地址。
struct Public {
    network: String,
    gateway: Ipv4Addr,
    forward: Ipv4Addr,
}

impl EntryContainers {
    /// `lanfwd`：`create` 时没有就建，否则不在就是空。
    pub(super) async fn lan_link(
        &self,
        project: &str,
        create: bool,
    ) -> Result<Option<LanLink>, EntryError> {
        let name = format!("{project}_lanfwd");
        if create && self.engine.inspect_network(&name).await?.is_none() {
            self.engine
                .create_network(&name, &self.lan_labels(), NetworkReach::Isolated)
                .await?;
        }
        let Some(layout) = self.reserved_layout(&name).await? else {
            return Ok(None);
        };
        let entry = last_usable(layout.subnet).ok_or_else(|| no_room(&name))?;
        Ok(Some(LanLink {
            network: name,
            entry,
            forward: Ipv4Addr::from(entry.to_bits() - 1),
        }))
    }

    /// 让局域网转发跟上：没有插件开局域网入口（`link` 为空）或者地址选不出来就删；状态还不确定（核心刚起来、
    /// 没检测过）时不动；否则按选中的地址建，地址、端口或放行的网段变了就重建。只动转发，不碰入口和连接器。
    pub(super) async fn follow_lan_forward(
        &self,
        project: &str,
        link: Option<&LanLink>,
    ) -> Result<(), EntryError> {
        let name = self.name(LAN_FORWARD_COMPONENT);
        let state = self.lan();
        let address = match (link, state.forward()) {
            (None, _) | (Some(_), Some(None)) => {
                self.engine.remove_container(&name).await?;
                return Ok(());
            }
            (Some(_), None) => return Ok(()),
            (Some(_), Some(Some(address))) => *address,
        };
        let link = link.expect("上面已经排除");
        let public = self.lan_public(project).await?;
        let entry_port = LAN_BIND
            .rsplit_once(':')
            .and_then(|(_, port)| port.parse().ok())
            .expect("局域网前端的写法是名字:端口");
        let config = render_forward(&Forward {
            listen: SocketAddrV4::new(public.forward, LISTEN_PORT),
            allow: address.subnet,
            deny: public.gateway,
            entry: SocketAddrV4::new(link.entry, entry_port),
        });
        let path = self.install_dir.join("entry").join(CONFIG);
        crate::install::write_private(&path, &config)
            .and_then(|()| crate::entry_compose::prepare_permissions(&self.install_dir))
            .map_err(|error| super::error::config_error("写局域网转发配置", error))?;
        let endpoints = vec![
            (
                public.network,
                Endpoint {
                    address: Some(public.forward.to_string()),
                    ..Endpoint::default()
                },
            ),
            (
                link.network.clone(),
                Endpoint {
                    address: Some(link.forward.to_string()),
                    aliases: Vec::new(),
                    // 内部网络：默认路由留在 `lanpub` 上，回包才回得到局域网。
                    gw_priority: Some(-1),
                },
            ),
        ];
        let ports = vec![(
            format!("{LISTEN_PORT}/tcp"),
            format!("{}:{}", address.publish, self.client_port),
        )];
        let command = ["haproxy", "-db", "-f"]
            .into_iter()
            .map(str::to_owned)
            .chain([format!("/config/{CONFIG}")])
            .collect();
        let binds = mounts(
            &self.install_dir.join("entry"),
            &[(CONFIG, &format!("/config/{CONFIG}"))],
        );
        let mut body = self.hardened(
            LAN_FORWARD_COMPONENT,
            command,
            binds,
            &ports,
            None,
            &endpoints,
        )?;
        let mut digest = std::collections::hash_map::DefaultHasher::new();
        config.hash(&mut digest);
        body.labels
            .get_or_insert_default()
            .insert(CONFIG_LABEL.to_owned(), format!("{:016x}", digest.finish()));
        let side = Side {
            component: LAN_FORWARD_COMPONENT,
            image: crate::entry_compose::HAPROXY_IMAGE,
            body,
            endpoints,
        };
        self.ensure(&side, project).await?;
        Ok(())
    }

    /// `lanpub`：关掉地址伪装的桥接，不在就建。
    async fn lan_public(&self, project: &str) -> Result<Public, EntryError> {
        let name = format!("{project}_lanpub");
        if self.engine.inspect_network(&name).await?.is_none() {
            self.engine
                .create_lan_network(&name, &self.lan_labels())
                .await?;
        }
        let layout = self
            .reserved_layout(&name)
            .await?
            .ok_or_else(|| EntryError::NetworkMissing(name.clone()))?;
        let gateway = layout
            .gateway
            .as_deref()
            .and_then(|gateway| gateway.parse().ok())
            .ok_or_else(|| EntryError::NetworkMissing(format!("{name} 的网关")))?;
        let forward = last_usable(layout.subnet).ok_or_else(|| no_room(&name))?;
        Ok(Public {
            network: name,
            gateway,
            forward,
        })
    }

    /// 网络的网段；没有核心留的地址（不是核心按地址池建的）时报错，网络不在时为空。
    async fn reserved_layout(&self, name: &str) -> Result<Option<Layout>, EntryError> {
        let Some(found) = self.engine.inspect_network(name).await? else {
            return Ok(None);
        };
        match layout_of(&found) {
            Some(layout) if layout.reserved => Ok(Some(layout)),
            _ => Err(no_room(name)),
        }
    }

    fn lan_labels(&self) -> std::collections::HashMap<String, String> {
        let mut labels = self.instance.core_labels();
        labels.insert(LABEL_MANAGED_BY.to_owned(), "core".to_owned());
        labels.insert(LABEL_COMPONENT.to_owned(), LAN_FORWARD_COMPONENT.to_owned());
        labels
    }
}

fn no_room(name: &str) -> EntryError {
    EntryError::NetworkMissing(format!("{name} 上核心留的固定地址"))
}
