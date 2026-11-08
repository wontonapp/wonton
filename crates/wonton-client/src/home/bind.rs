//! 本设备身份的 endpoint：配对、配对后建连、换服务器和重启后重连都从这里绑定。

use iroh::endpoint::{Builder, QuicTransportConfig, presets};
use iroh::{Endpoint, RelayMode};
use wonton_l10n::{Lang, l10n};
use wonton_tunnel::relays::{Relay, relay_map};

use super::{HomeLink, Network};
use crate::l10n::CATALOG;

impl HomeLink {
    /// 绑定经 [`wonton_tunnel::off_workers`] 跑在阻塞线程池里：iroh 的绑定里有整段同步阻塞的网卡枚举，
    /// 在作用域运行时的 worker 上跑会把 worker 和外层的时限一起拖住（Android 上配对因此一直转圈）。
    /// 失败时返回给人看的原因。
    pub(super) async fn bind_endpoint(
        &self,
        relays: &[Relay],
        tiers: usize,
        lang: Lang,
    ) -> Result<Endpoint, String> {
        let builder = self.endpoint_builder(relays, tiers);
        #[cfg(feature = "test-hooks")]
        let hook = self
            .test_hooks
            .bind
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        let bound = wonton_tunnel::off_workers(move || {
            #[cfg(feature = "test-hooks")]
            if let Some(hook) = hook {
                hook();
            }
            builder.bind()
        })
        .await;
        let error = match bound {
            Ok(Ok(endpoint)) => return Ok(endpoint),
            Ok(Err(error)) => error.to_string(),
            Err(error) => error.to_string(),
        };
        Err(l10n!(
            CATALOG,
            lang,
            "home-pairing-bind-failed",
            error = error
        ))
    }

    /// `relays` 是连接信息或配对码带来的中继表，`tiers` 是要放进 endpoint 的档数：App 的隧道 endpoint
    /// 只放最优先的一档（ADR-0048），配对一次成、等不了回退，放全部档。连接信息里一个中继都没有时才用内置 n0。
    ///
    /// 关掉 GSO：网卡不支持时（实测 Android 模拟器的 virtio 网卡；内核对没有校验和卸载的网卡和 IPsec 都这样）
    /// 多包的发送报 EIO。noq-udp 随后关掉这个 socket 的 GSO，但 iroh 1.4.1 的发送端把分段上限存在建连那一刻，
    /// 这条连接之后的多包发送每次都 EIO，错误还被吞掉：单包的心跳和确认照常往返，连接看着活着，请求却卡到
    /// 连接被换掉（#154）。手机上的流量用不上 GSO 省下的那点 CPU。
    fn endpoint_builder(&self, relays: &[Relay], tiers: usize) -> Builder {
        let builder = match (self.network, relays.is_empty()) {
            (Network::Internet, false) => Endpoint::builder(presets::N0)
                .relay_mode(RelayMode::Custom(relay_map(relays, tiers))),
            (Network::Internet, true) => Endpoint::builder(presets::N0),
            #[cfg(feature = "test-hooks")]
            (Network::TestRelay, _) => Endpoint::builder(presets::Minimal)
                .clear_ip_transports()
                .bind_addr((std::net::Ipv4Addr::LOCALHOST, 0))
                .expect("fixed loopback bind is valid")
                .relay_mode(RelayMode::Custom(relay_map(relays, tiers)))
                .ca_tls_config(iroh::tls::CaTlsConfig::insecure_skip_verify()),
            (Network::DirectOnly, _) => Endpoint::builder(presets::Minimal),
        };
        let transport = QuicTransportConfig::builder()
            .enable_segmentation_offload(false)
            .build();
        builder
            .secret_key(self.device_key.clone())
            .transport_config(transport)
    }
}

/// 只给测试：绑定里的阻塞段，和配对的两道时限。
#[cfg(feature = "test-hooks")]
#[derive(Default)]
pub(crate) struct TestHooks {
    /// 在阻塞线程里、iroh 绑定之前调用：一直不返回就是卡住的网卡枚举。
    pub(crate) bind: std::sync::Mutex<Option<std::sync::Arc<dyn Fn() + Send + Sync>>>,
    /// 配对交换的时限和看门线程的时限，替掉生产的那两份。
    pub(crate) pair_deadlines: std::sync::Mutex<Option<(std::time::Duration, std::time::Duration)>>,
}
