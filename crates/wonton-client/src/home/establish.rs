//! 换上一条新连接：绑定 endpoint、起隧道客户端和本地代理。建不起来时如实报连不上，按退避自己再试，
//! 主人点“重试”时马上再试（ADR-0032）。

use std::net::Ipv4Addr;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use wonton_l10n::l10n;
use wonton_protocol::{ConnectionCheck, HomeFailure, HomeStatus, ProxySettings};
use wonton_tunnel::relays::{FALLBACK_AFTER, RelayTable, spawn_tier_fallback};
use wonton_tunnel::{LocalProxy, ProxyAccess, ProxyCredential, TunnelClient};

use super::bind::BindError;
use super::check::keep_checking;
use super::link::Link;
use super::ticket::Ticket;
use super::{HomeLink, Network, refresh};
use crate::l10n::{CATALOG, language};

/// 建连时绑定 endpoint 的时限。正常的绑定不到一秒（arm64 包在 x86 模拟器上经翻译跑也一样）；卡住的绑定
/// 等多久都不会回来（见 [`HomeLink::bind_endpoint`]），到点放弃、换一次新的绑定。
const BIND_DEADLINE: Duration = Duration::from_secs(10);
/// 建不起来之后第一次再试的间隔，之后每次翻倍。和隧道客户端建连的退避同一个量级。
const RETRY_FIRST: Duration = Duration::from_secs(1);
const RETRY_MAX: Duration = Duration::from_secs(30);

impl HomeLink {
    pub(super) fn start(self: &Arc<Self>, text: String, ticket: Ticket) {
        *self.lock_ticket() = Some(text);
        *self
            .core_url
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(ticket.core_url());
        self.lock_tracker().new_server();
        self.carried_disconnects.store(0, Ordering::SeqCst);
        *self.lock_unlinked() = None;
        self.view.send_modify(|view| {
            view.server_id = Some(ticket.server_id().to_string());
            view.web_url = ticket.web_url().map(str::to_owned);
            view.status = HomeStatus::Connecting;
            view.path = None;
            view.proxy = None;
            view.notice = None;
            view.just_recovered = false;
        });
        self.runtime.spawn(self.clone().establish(ticket));
    }

    /// 换上一条新连接，换不上就一直按退避再试，直到换上或者又有一次更新的 `establish` 接手。
    ///
    /// 新连接换上之前，`link` 锁一直拿着：这期间要连接的调用方等着拿新的，不会先拿到空的当作没配对。
    /// 锁里只做有时限的事：旧连接的关闭放到后台（卡死的 endpoint 关起来也可能不返回），绑定有
    /// [`BIND_DEADLINE`]。两次尝试之间锁是放开的，这时连接确实不在，调用方如实拿到空的。
    pub(super) async fn establish(self: Arc<Self>, ticket: Ticket) {
        let generation = self.establishing.fetch_add(1, Ordering::SeqCst) + 1;
        let mut delay = self.establish_timing().1;
        let mut failures = 0u32;
        loop {
            let message = {
                let mut current = self.link.lock().await;
                if self.establishing.load(Ordering::SeqCst) != generation {
                    return;
                }
                if let Some(old) = current.take() {
                    *self.lock_lifecycle_client() = None;
                    let stats = old.client.stats();
                    self.carried_disconnects
                        .fetch_add(stats.disconnects, Ordering::SeqCst);
                    self.runtime.spawn(old.close());
                }
                match self.open(&ticket).await {
                    Ok(link) => {
                        let client = link.client.clone();
                        *self.lock_unlinked() = None;
                        *current = Some(link);
                        drop(current);
                        // 主动建连：壳打开网页之前，状态就已经是准的。
                        self.check(&client).await;
                        return;
                    }
                    Err(message) => message,
                }
            };
            failures = failures.saturating_add(1);
            tracing::info!(
                failures,
                ?delay,
                "could not set up the connection: {message}"
            );
            *self.lock_unlinked() = Some((message, failures));
            self.republish_unlinked();
            // 退避睡着时主人点“重试”、回到前台、网络回来，都马上再试。
            tokio::select! {
                () = tokio::time::sleep(delay) => {}
                () = self.establish_wake.notified() => {}
            }
            let mut background = self.background.subscribe();
            if background.wait_for(|background| !background).await.is_err() {
                return;
            }
            delay = (delay * 2).min(RETRY_MAX);
        }
    }

    /// 连接还没换上（上一次建连失败、在等下一次）时把它叫醒，马上再试。没有在等的时候留一个许可，
    /// 下一次失败后立刻再试一次，不白等一轮退避。
    pub(super) fn wake_establish(&self) {
        self.establish_wake.notify_one();
    }

    /// 绑定的时限和第一次再试的间隔。
    fn establish_timing(&self) -> (Duration, Duration) {
        #[cfg(feature = "test-hooks")]
        if let Some(timing) = *self
            .test_hooks
            .establish_timing
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
        {
            return timing;
        }
        (BIND_DEADLINE, RETRY_FIRST)
    }

    fn lock_unlinked(&self) -> std::sync::MutexGuard<'_, Option<(String, u32)>> {
        self.unlinked
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// 连接没建起来、没有隧道客户端可跟时，把“连不上”照常算成 ViewModel：和 [`HomeLink::publish`]
    /// 走同一个判断，诊断窗口、提示和重试入口都一样，断开次数接着算。连接在时什么也不做。
    pub(super) fn republish_unlinked(&self) {
        let Some((message, failures)) = self.lock_unlinked().clone() else {
            return;
        };
        let now = Instant::now();
        let status = HomeStatus::Disconnected {
            reason: HomeFailure::TransportUnavailable,
            message: message.clone(),
        };
        let mut tracker = self.lock_tracker();
        tracker.observe(&status, ConnectionCheck::Unknown, failures, now);
        let notice = tracker.notice(&status, now);
        let network = tracker.network();
        let relay = tracker.relay();
        let server = tracker.server_check(&status, now);
        drop(tracker);
        let disconnects = self.carried_disconnects.load(Ordering::SeqCst);
        self.view.send_modify(|view| {
            view.status = status;
            view.notice = notice;
            view.path = None;
            view.proxy = None;
            view.just_recovered = false;
            view.details = wonton_protocol::ConnectionDetails {
                network,
                relay,
                server,
                disconnects,
                failed_dials: failures,
                last_connected_at_ms: view.details.last_connected_at_ms,
                last_failure: Some(message),
                ..Default::default()
            };
        });
    }

    async fn open(self: &Arc<Self>, ticket: &Ticket) -> Result<Link, String> {
        let started = Instant::now();
        let lang = language();
        let deadline = self.establish_timing().0;
        let endpoint = match self.bind_endpoint(ticket.relays(), 1, lang, deadline).await {
            Ok(endpoint) => endpoint,
            Err(BindError::Failed(message)) => return Err(message),
            Err(BindError::TimedOut) => {
                return Err(l10n!(
                    CATALOG,
                    lang,
                    "home-bind-timeout",
                    seconds = deadline.as_secs()
                ));
            }
        };
        tracing::info!(elapsed = ?started.elapsed(), "bound the endpoint");
        // 只建最优先的一档（ADR-0048）；掉档回退由这个任务盯着家中继状态做。DirectOnly 不连中继，
        // 连接信息里一个中继都没有时没有表，也不回退。
        if self.network == Network::Internet && !ticket.relays().is_empty() {
            spawn_tier_fallback(
                endpoint.clone(),
                RelayTable::from_relays(ticket.relays().to_vec()),
                FALLBACK_AFTER,
                None,
            );
        }
        let client = TunnelClient::new(endpoint.clone(), ticket.server_addr());
        let credential = ProxyCredential::generate();
        let domain = ticket.base_domain().to_owned();
        let access = ProxyAccess::Credential {
            credential: credential.clone(),
            domain: domain.clone(),
        };
        let proxy = LocalProxy::bind(client.clone(), (Ipv4Addr::LOCALHOST, 0).into(), access)
            .await
            .map_err(|e| {
                l10n!(
                    CATALOG,
                    lang,
                    "home-proxy-start-failed",
                    error = e.to_string()
                )
            })?;
        let registration = self
            .hub
            .register(proxy.local_addr(), credential, domain.clone())
            .map_err(|_| "proxy route registration failed".to_owned())?;
        let credential = registration.credential();
        {
            let mut current = self.lock_lifecycle_client();
            client.set_suspendable(self.app_backgrounded.load(Ordering::SeqCst));
            *current = Some(client.clone());
        }
        let settings = ProxySettings {
            host: Ipv4Addr::LOCALHOST.to_string(),
            port: registration.local_addr().port(),
            username: credential.username().to_owned(),
            password: credential.password().to_owned(),
            domain,
        };
        self.view
            .send_modify(|view| view.proxy = Some(settings.clone()));
        let watcher = self.runtime.spawn(refresh::follow_connection(
            Arc::downgrade(self),
            endpoint.clone(),
            client.clone(),
            settings,
        ));
        let checker = self.runtime.spawn(keep_checking(
            Arc::downgrade(self),
            endpoint.clone(),
            client.clone(),
        ));
        Ok(Link {
            endpoint,
            client,
            proxy,
            registration,
            watcher,
            checker,
        })
    }
}
