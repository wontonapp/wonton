//! 本设备身份的 endpoint：配对、配对后建连、换服务器和重启后重连都从这里绑定。

use std::time::Duration;

use iroh::endpoint::{Builder, QuicTransportConfig, presets};
use iroh::{Endpoint, RelayMode};
use wonton_l10n::{Lang, l10n};
use wonton_tunnel::relays::{Relay, RelayTable, relay_map};

use super::{HomeLink, Network};
use crate::l10n::CATALOG;

/// 一次绑定没有成：失败带给人看的原因；超时由调用方按自己的场景说。
#[derive(Debug)]
pub(super) enum BindError {
    Failed(String),
    TimedOut,
}

impl HomeLink {
    /// 绑定经 [`wonton_tunnel::off_workers`] 跑在阻塞线程池里：iroh 的绑定里有整段同步阻塞的网卡枚举，
    /// 在作用域运行时的 worker 上跑会把 worker 和外层的时限一起拖住（Android 上配对因此一直转圈）。
    ///
    /// 到 `deadline` 还没绑好就放弃，交给调用方再来一次。iroh 的网络监视在 Android 上 fork 执行 `ip route`
    /// （netwatch）。x86 模拟器经 ndk_translation 跑 arm64 包时，fork 出的子进程约一半卡死在翻译层的锁上，
    /// 父进程停在同步的 `spawn` 里等它 exec，绑定永远不返回；这个阻塞线程和卡死的子进程都收不回来，留在那里，
    /// 再绑一次多半就好。真机上没有翻译层，但同样不该无限期地等一次绑定：时限是建连能自己重试的前提。
    pub(super) async fn bind_endpoint(
        &self,
        relays: &[Relay],
        tiers: usize,
        lang: Lang,
        deadline: Duration,
    ) -> Result<Endpoint, BindError> {
        let builder = self.endpoint_builder(relays, tiers);
        #[cfg(feature = "test-hooks")]
        let hook = self
            .test_hooks
            .bind
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        let bind = wonton_tunnel::off_workers(move || {
            #[cfg(feature = "test-hooks")]
            if let Some(hook) = hook {
                hook();
            }
            builder.bind()
        });
        // 到点放弃时，绑定本身还在跑：日志里记下它之后多久才回来。看得出时限是不是掐了一个快好的绑定，
        // 还是真卡住的那种（真卡住就一直不回来）。
        let started = std::time::Instant::now();
        let mut bind = Box::pin(bind);
        let error = match tokio::time::timeout(deadline, &mut bind).await {
            Ok(Ok(Ok(endpoint))) => return Ok(endpoint),
            Err(_) => {
                tokio::spawn(async move {
                    let elapsed = started.elapsed();
                    match bind.await {
                        Ok(Ok(_)) => {
                            tracing::info!(?elapsed, "an abandoned bind only finished now")
                        }
                        Ok(Err(error)) => {
                            tracing::info!(?elapsed, error = %error, "an abandoned bind only failed now");
                        }
                        Err(_) => {}
                    }
                });
                return Err(BindError::TimedOut);
            }
            Ok(Ok(Err(error))) => error.to_string(),
            Ok(Err(error)) => error.to_string(),
        };
        Err(BindError::Failed(l10n!(
            CATALOG,
            lang,
            "home-pairing-bind-failed",
            error = error
        )))
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
            (Network::Internet, true) => Endpoint::builder(presets::N0).relay_mode(
                RelayMode::Custom(relay_map(RelayTable::builtin_n0().relays(), tiers)),
            ),
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
    /// 配对的每一段时限（绑定、一次交换、一次确认）和看门线程的时限，替掉生产的那两份。
    pub(crate) pair_deadlines: std::sync::Mutex<Option<(std::time::Duration, std::time::Duration)>>,
    /// 配对后建连的绑定时限和建不起来后第一次再试的间隔，替掉生产的那两份。
    pub(crate) establish_timing:
        std::sync::Mutex<Option<(std::time::Duration, std::time::Duration)>>,
    /// 冷启动的建连时限和之后的建连时限，替掉生产的那两份。
    pub(crate) cold_allowance: std::sync::Mutex<Option<(std::time::Duration, std::time::Duration)>>,
}
