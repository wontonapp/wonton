//! 假数据入口的启动和 UI 测试专用钩子：`new_ui_fake` 建一个没有后端的核心、把订阅换成写死的值，
//! 再给界面测试留几个只认假核心的入口。

use std::sync::Arc;
use std::sync::atomic::Ordering;

use tokio::sync::watch;

use wonton_protocol::{
    CaaState, CertificateState, ClientsDomainCertificate, ClientsDomainProvider,
    ClientsDomainState, ClientsDomainViewModel, CoreError, LanRecordState, PagesViewModel,
    TimelineTarget,
};

use super::super::ClientCore;
use super::super::ui_fake_devices;
use super::super::ui_fake_settings;
use super::chat::fake_chat;
use super::home::{fake_home, fake_servers};
use super::onboarding::fake_onboarding;
use super::pages::fake_pages;
use super::plugins::fake_plugins;
use crate::scope::Views;

#[uniffi::export]
impl ClientCore {
    /// UI 测试专用的启动入口：首屏直接是主界面，内容由 `seed_ui_fake` 写死。壳只在 Debug 里调。
    #[uniffi::constructor]
    pub fn new_ui_fake(data_dir: String) -> Result<Arc<Self>, CoreError> {
        let core = Self::new(data_dir)?;
        core.ui_fake.store(true, Ordering::SeqCst);
        core.seed_ui_fake();
        Ok(core)
    }

    /// UI 测试专用：让一条时间线像真核心那样自己结束（房间关了、旁聊根被撤回），等着的 `next()` 马上回 `None`。
    /// 返回本来订没订着。
    pub fn ui_fake_end_timeline(&self, target: TimelineTarget) -> bool {
        if !self.ui_fake.load(Ordering::SeqCst) {
            return false;
        }
        self.ui_fake_timelines.end(&target)
    }

    /// UI 测试专用：恢复页面初态并写出 fixture；只接受 ui-fake 核心。
    pub fn reset_ui_fake_pages_fixture(&self, path: String) -> Result<(), CoreError> {
        if !self.ui_fake.load(Ordering::SeqCst) {
            return Err(CoreError::InvalidInput {
                reason: "page fixtures require a ui-fake core".into(),
            });
        }
        let views = self.views();
        let pages = fake_pages(views.pages.borrow().clone());
        let fixture = serde_json::to_vec(&pages).map_err(|error| CoreError::InvalidInput {
            reason: format!("cannot encode page fixture: {error}"),
        })?;
        std::fs::write(path, fixture).map_err(|error| CoreError::InvalidInput {
            reason: format!("cannot write page fixture: {error}"),
        })?;
        views.pages.send_replace(pages);
        Ok(())
    }

    /// UI 测试专用：从 fixture 发布页面状态帧，不执行生产操作。
    pub fn apply_ui_fake_pages_fixture(&self, path: String) -> Result<(), CoreError> {
        if !self.ui_fake.load(Ordering::SeqCst) {
            return Err(CoreError::InvalidInput {
                reason: "page fixtures require a ui-fake core".into(),
            });
        }
        let fixture = std::fs::read(path).map_err(|error| CoreError::InvalidInput {
            reason: format!("cannot read page fixture: {error}"),
        })?;
        let pages: PagesViewModel =
            serde_json::from_slice(&fixture).map_err(|error| CoreError::InvalidInput {
                reason: format!("cannot decode page fixture: {error}"),
            })?;
        self.views().pages.send_replace(pages);
        Ok(())
    }
}

impl ClientCore {
    fn seed_ui_fake(&self) {
        // 换成独立的一组发送端：真实作用域读盘算完的首帧还会发出来，接在同一组通道上就会把假数据盖回去。
        let current = self.views();
        let views = Views {
            home: watch::Sender::new(fake_home(current.home.borrow().clone())),
            chat: watch::Sender::new(fake_chat(current.chat.borrow().clone())),
            plugins: watch::Sender::new(fake_plugins(current.plugins.borrow().clone())),
            pages: watch::Sender::new(fake_pages(current.pages.borrow().clone())),
            transfers: watch::Sender::new(current.transfers.borrow().clone()),
            media: crate::media_link::MediaViews {
                media: watch::Sender::new(current.media.media.borrow().clone()),
                playback: watch::Sender::new(current.media.playback.borrow().clone()),
            },
            onboarding: watch::Sender::new(fake_onboarding(current.onboarding.borrow().clone())),
            peers: watch::Sender::new(current.peers.borrow().clone()),
            friends: watch::Sender::new(current.friends.borrow().clone()),
            family: watch::Sender::new(current.family.borrow().clone()),
            photos: watch::Sender::new(super::super::ui_fake_photos::fake_photos()),
            bot_contacts: watch::Sender::new(current.bot_contacts.borrow().clone()),
            clients_domain: watch::Sender::new(fake_clients_domain()),
            // 假数据下设置页带着当前值：真设置页的构建循环订阅着被丢弃的旧接收器，开起来会停在
            // 未配对的状态，设置里的“添加设备”走不通。
            settings: watch::Sender::new(ui_fake_settings::fake_settings(
                current.settings.borrow().clone(),
            )),
            confirmations: watch::Sender::new(ui_fake_devices::confirmations(
                current.confirmations.borrow().clone(),
            )),
            agent_chains: watch::Sender::new(current.agent_chains.borrow().clone()),
            agent_stops: watch::Sender::default(),
        };
        drop(current);
        self.servers.seed_ui_fake(fake_servers());
        *self
            .views
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = views;
    }
}

/// 假设置页：已配好一个后缀、deSEC 一项，证书和 CAA 都停在需要处理的状态。
fn fake_clients_domain() -> ClientsDomainViewModel {
    ClientsDomainViewModel {
        state: ClientsDomainState::Ready,
        suffix: Some("wonton.lan".into()),
        suffix_editable: true,
        providers: vec![
            ClientsDomainProvider {
                code: Some("desec".into()),
                name: "deSEC".into(),
                recommended: true,
                signup_url: Some("https://desec.io".into()),
                credential_vars: vec!["DESEC_TOKEN".into()],
            },
            ClientsDomainProvider {
                code: None,
                name: String::new(),
                recommended: false,
                signup_url: None,
                credential_vars: Vec::new(),
            },
        ],
        selected_provider: Some("desec".into()),
        credential_vars: vec!["DESEC_TOKEN".into()],
        terms_accepted: true,
        certificate: ClientsDomainCertificate {
            state: CertificateState::Expired {
                not_after_ms: 1_760_000_000_000,
                error: "acme: expired".into(),
            },
            retryable: true,
            details: Some("acme: certificate expired".into()),
        },
        caa: CaaState::Missing,
        caa_record: Some(
            "wonton.lan. CAA 0 issue \"letsencrypt.org; accounturi=https://acme-v02.api.letsencrypt.org/acme/acct/1; validationmethods=dns-01\""
                .into(),
        ),
        lan_record: LanRecordState::Missing,
        lan_record_name: Some("*.lan.wonton.lan".into()),
        lan_target: Some("192.0.2.10".into()),
        ..ClientsDomainViewModel::default()
    }
}

#[cfg(test)]
mod tests {
    use super::fake_clients_domain;
    use wonton_protocol::{CaaState, CertificateState, LanRecordState};

    /// 假页面的每条都填上：界面测试要看到证书失败、CAA 和 `*.lan` 记录，不能是默认值。
    #[test]
    fn the_fake_domain_page_shows_every_row() {
        let view = fake_clients_domain();
        assert!(matches!(
            view.state,
            wonton_protocol::ClientsDomainState::Ready
        ));
        assert!(matches!(
            view.certificate.state,
            CertificateState::Expired { .. }
        ));
        assert!(view.certificate.retryable);
        assert!(view.certificate.details.is_some());
        assert_eq!(view.caa, CaaState::Missing);
        assert!(view.caa_record.is_some());
        assert_eq!(view.lan_record, LanRecordState::Missing);
        assert!(view.lan_record_name.is_some());
        assert!(view.lan_target.is_some());
    }
}
